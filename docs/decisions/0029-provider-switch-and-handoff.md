# 0029 — Switching provider in a thread, with a bounded handoff

- **Status:** Accepted
- **Date:** 2026-11-24

## Context

A thread's provider is fixed at `thread.create`. Users want to move a conversation to
another agent: a cheaper one for routine work, a different one when the first is stuck
or out of quota. `thread.update` already changes `model` and `permissionMode`, ending the
idle session so the next turn resumes with the new settings.

A provider's native session id cannot cross providers: the new agent has never seen the
conversation. It needs the history in some form. The same need appears for forks (a new
thread with a copy of the history and no session) and for threads whose session id was
lost.

Options for passing history:

1. Prepend it to the first prompt's text in the kernel. Works for every provider, but
   each provider cannot place it well (system prompt, a separate message).
2. A typed field on the input that providers render. Lets a provider put it where its
   agent works best; needs a new WIT version.
3. A summary written by the old agent. Shorter and focused, but costs a turn and can
   omit details.

## Decision

- **Switch.** `thread.update` accepts `provider` (with an optional `model`, defaulting to
  the provider's first) and `handoff` (`transcript`, the default, or `summary`) while the
  thread is idle (`conflict` otherwise). The kernel ends the session, clears the stored
  `resume` id and appends `thread.switched { from { provider, model }, to { provider,
  model }, handoff }`.
- **Attribution.** `turn.started` gains `provider`, `model` and `purpose?`, so every turn
  keeps its author after later switches. Clients label replies by turn.
- **Context.** The kernel builds a bounded handoff context from the thread's event log:
  `from_provider`, `from_model`, `messages` (`{ role, text, turn }`, user and assistant text
  only, without tool calls, reasoning, attachment bytes or `handoff_summary` turns),
  `files_changed` (paths from `file.changed`), `truncated` and `summary`. It holds at most
  the newest 200 messages and 64 KiB encoded; when it is cut, `truncated` is set and the
  rendered text says so.
- **Rendering.** `sdk/rust-provider` (`handoff::render`) and the kernel share the exact
  prompt-preamble format. For `kumikit:provider@0.2.0` providers the kernel prepends that
  rendered preamble to `user-input.text` while keeping the thread's `turn.started` event
  clean, so every provider works without a WIT breaking change.
- **Delivery.** The handoff is pending until a turn on the new provider ends `done` and the
  provider has returned a session id; then the kernel appends `handoff.delivered`. A
  cancelled or failed first turn keeps it pending, so the next message carries it again.
- **Summary mode.** Before switching, the kernel runs one turn on the outgoing provider
  with a fixed prompt asking for a handoff note (goal, decisions, current state, open
  items, files), with `turn.started { purpose: "handoff_summary" }`. Its final text
  becomes `context.summary`; the bounded transcript is still sent. Clients show that turn
  collapsed and labelled. If it fails, the switch goes ahead in `transcript` mode and
  says so.
- **Forks and lost sessions** use the same context: a thread with history and no `resume`
  id gets a pending handoff for its next turn.

## Consequences

- Any pair of providers can hand off; the kernel still names none.
- The new agent sees text, not tool state; it may re-read files. Users are told what was
  carried forward: the UI shows the exact context.
- Older clients ignore the new `turn.started` fields and the new events.
- Tests: the bounds and truncation note; a Claude Code recording, then a switch to a fake
  ACP agent whose first prompt holds the earlier exchange; a cancelled first turn resends
  it; summary mode with a failing summary turn falls back to transcript.
