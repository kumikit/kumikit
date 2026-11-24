# 0027 — A durable thread queue, held after stops, and deferred messages

- **Status:** Proposed
- **Date:** 2026-11-24

## Context

A message sent during a turn waits in an in-memory `VecDeque` in the kernel
([03](../spec/03-kernel.md#thread-events)). It is lost if the core stops, and
`thread.cancel` drops it. The client shows queued messages as plain bubbles and cannot
edit, reorder or remove them. Users lose typed work when they press Stop to correct the
agent, which is when they most need the queue kept.

Users also want a message to wait for something: a time ("check the deploy in 20
minutes"), another thread's turn ("when the refactor thread finishes, review it"), or a
command ("when `cargo build --release` passes", "when CI is green"). Agents need the same
to wake themselves after starting slow work. Cron schedules owned by plugins
([ADR 0024](0024-schedules.md)) cover recurring jobs, not these one-off, thread-bound
waits.

Options for where the queue lives:

1. In the client. Lost when the window closes, not shared between clients, and wrong for
   remote cores that keep running.
2. In the kernel, in memory. Today's behavior.
3. In the kernel, in SQLite, with changes recorded as thread events. Survives restarts,
   every client sees the same queue, and replay from a cursor stays exact.

Options for waits: a separate scheduler plugin, or queue entries with a trigger. A
plugin would need a long-lived instance to watch other threads (against "lazy by
default"), and its waiting messages would sit outside the queue the user sees.

## Decision

- **Storage.** Queued messages are rows in a kernel table `queued_messages` (`id` ULID,
  `thread`, `position`, `input` JSON, `trigger` JSON or null, `from`, `created`,
  `updated`), written by the store's writer thread in the same transaction as their
  events.
- **Events.** Queue changes are thread events, so the log stays the record:
  `queue.added { entry, front? }`, `queue.updated { entry }`, `queue.removed { entry,
  reason }` (`removed`; later steps add reasons), `queue.moved { order }`,
  `queue.held { reason }` (`stopped`, `failed`, `restarted`; later `limited`) and
  `queue.resumed`. An entry that starts leaves the queue through `turn.started { entry }`
  in the same transaction, so a crash cannot lose or repeat it. `thread.get` returns
  `queue` and `queueHold?`.
- **Methods.** Each names the thread: `thread.queue.update { thread, entry, input }`,
  `thread.queue.remove { thread, entry }`, `thread.queue.move { thread, entry, before? }`,
  `thread.queue.sendNow { thread, entry }` and `thread.queue.resume { thread }`.
  `thread.send` keeps `turn` and `queued` and adds `entry` when it queued.
- **Hold rules.** The next entry starts automatically only after a turn ends `done`.
  After `cancelled` or `failed`, and for every non-empty queue at startup, the queue is
  held and waits for `thread.queue.resume`, `sendNow` or a new `thread.send` (which
  resumes it). `thread.cancel` no longer drops queued messages. This changes the rule in
  [02](../spec/02-protocol.md#kernel-methods) and [03](../spec/03-kernel.md#thread-events).
- **Deferred messages.** `thread.defer { thread, input, trigger }` adds an entry that
  waits for its trigger, then joins the queue (it never steers a running turn):
  - `at { time }` (RFC 3339); one that passed while the core was down fires once at
    startup.
  - `after_turn { thread, onlyOnSuccess? }`: when that thread's running or next turn
    ends. A chain of `after_turn` entries that leads back to its own thread is refused
    with `invalid_params`.
  - `after_command { argv, cwd?, timeoutS }`: the kernel runs the command through its
    process supervisor in the thread's workspace or worktree; on exit the message is
    sent with the exit code and the last 8 KiB of output appended.
  - `poll_command { argv, cwd?, everyS, maxTries }`: runs the command every `everyS`
    seconds until it exits 0, then sends the message; after `maxTries` it sends the
    message with the last failure instead.
  Commands do not survive a restart: their entries are held with `restarted` and offer
  to run again. The fired message carries a one-line trailer saying what woke it, and
  `turn.started.from` names the source.
- **Who may defer.** A client acts for the user and may set any trigger. A plugin needs
  the client operation `threads.defer`; a command trigger also needs `spawn` covering the
  program and an approval of kind `command` unless the thread is in `full_access`. At most
  10 pending plugin-set deferred messages per thread.
- **Recurring jobs** stay with plugin schedules ([ADR 0024](0024-schedules.md)).

## Consequences

- Stop is safe: typed follow-ups survive it, and a restart.
- Queue edits grow the thread log a little; compaction may fold them later without
  changing what a replay shows.
- The protocol gains a minor version with these methods and events. Clients from before
  it see a held queue as "nothing happens after Stop" until they send again, which
  resumes it.
- The kernel runs commands that a user or a permitted plugin asked for. They go through
  the process supervisor with its environment and kill rules; no new capability bypasses
  permissions.
- The kernel arms one timer for the earliest `at` or `poll_command` entry and none when
  there are none, keeping within the idle footprint budget.
- Tests: restart keeps a held queue in order; cancel holds; `sendNow` on a running
  non-steerable turn cancels it and sends next; each trigger with the deterministic
  clock; loop refusal; the denied paths for `threads.defer` and command triggers.
