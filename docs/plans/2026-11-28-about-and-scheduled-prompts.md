# Plan — Settings → About, and scheduled prompts from the composer

Three M9 follow-ups:

1. The Updates page shows the version in its status line with the brackets pushed apart
   ("( v0.1.0-m9.2 )"): the status box is a flex row with `justify-content:
   space-between`, so each text run and the `<code>` become separate flex items.
2. **Settings → About** replaces **Settings → Updates**. It keeps everything Updates had
   and adds **Resource usage** for each connected core: the core's memory, its active
   agents, the processes it runs and the size of its database.
3. A **Schedule** button on the composer opens a popover to send a message later, once or
   on repeat: presets (in 15 min, in 1 h, tonight 21:00, tomorrow 09:00), a date and time,
   and **Repeat** (once, every hour, every day, weekdays, every week). "More options…"
   keeps today's command triggers (after a command exits, when a command passes).

## Decisions

- **Repeating messages are deferred messages.** A deferred message's `at` trigger gains an
  optional `repeat` rule (`{ cron, timeZone? }`). When it fires, the kernel queues the next
  one at the rule's next time after the one that fired, with the same text, so the thread
  is woken again in its own context. Removing the queued entry stops the series. The
  kernel already parses cron and time zones for plugin schedules, so nothing new is
  needed, and no plugin is involved. A repeating message is not offered to plugins through
  WIT: the WIT trigger type is published and stays as it is.
- **`core.stats`** is a new method next to `core.activity`: `memoryBytes?` (the core
  process's resident memory, absent where the OS cannot say), `databaseBytes` (the
  database file with its WAL and shared-memory files), `activeTurns`, `processes`
  (programs the core's supervisor runs now, such as agent CLIs) and `uptimeSeconds`.
  Memory comes from the `memory-stats` crate (MIT OR Apache-2.0), which wraps the
  per-platform calls; the workspace denies `unsafe` code, so it is not written here.
- Both are protocol minor additions (0.14), so no ADR.
- About reads the stats when it is shown and on **Refresh**; nothing polls.
- The `updates` page id still opens the page (the status bar's update dot and the update
  notice link to it), now as About.

## Steps

- [x] **1. Bracket fix.** Status messages in `UpdatesSettings.svelte` are wrapped in one
  element, so the flex layout no longer splits the sentence. Vitest checks the text.
- [x] **2. Protocol 0.14.** `core.stats` (`CoreStatsResult`), and `QueueTrigger::At`
  gains `repeat?` (`QueueRepeat { cron, timeZone? }`). Fixtures, generated TypeScript,
  version bump, `02-protocol.md` ("Additions in 0.14", the method and trigger rows).
- [x] **3. Kernel.** `Kernel::stats()` (active turns, supervisor processes, database
  bytes read with `spawn_blocking`). Deferred messages: `repeat` is validated (cron and
  time zone parse; a `repeat` must yield a next time); on firing, the next occurrence is
  queued as a new deferred entry. Tests: stats; a repeating message fires and queues the
  next one at the right time (with a time zone); removing it stops the series; a bad rule
  is `invalid_params`. `03-kernel.md` (the queue).
- [x] **4. Core.** `core.stats` in `rpc.rs`, with memory and uptime. Integration test
  through the core binary.
- [x] **5. Settings → About.** A kernel-owned `AboutSettings.svelte`: the version and
  install kind, the Updates section as today, and Resource usage per connected core
  (memory, active agents, processes, database size, uptime), with Refresh. The navigation
  shows About instead of Updates (with the update dot). Vitest for the page; the
  Playwright updates test and the axe sweep follow the rename. `06-client.md`.
- [x] **6. Schedule popover in `ui-chat`.** The composer's clock button opens a popover
  anchored to it (Escape and clicking outside close it; focus returns to the button):
  message (the draft, or "Continue where you left off." when the draft is empty),
  presets, date and time, Repeat, "More options…" with the command triggers, Cancel and
  Schedule. It sends `thread.defer` with the trigger (and `repeat`). The queue panel names
  a repeating entry's rule ("every weekday at 09:00"). Theme tokens only; works with
  reduced motion; IME-safe keys. Vitest for the cron each Repeat choice builds and for
  the popover; Playwright schedules a message and sees it queued.
- [x] **7. Verify.** Docs, plugin names, keys, fmt, clippy (host and wasm), `cargo test
  --workspace`, `cargo deny check licenses bans sources` (new crate), protocol tests and
  `tsc`, `npm run check`, `npm test`, bundle, Playwright Chromium, desktop e2e in the
  container.
