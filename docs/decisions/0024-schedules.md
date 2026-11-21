# 0024 — Schedules: kernel-owned timers that wake plugins

- **Status:** Accepted
- **Date:** 2026-11-19

## Context

M5 adds scheduled work (scheduled prompts; any plugin that runs periodically). The
manifest already accepts `schedule:<cron>` activation triggers, but nothing runs them. A
plugin cannot keep its own timer: idle instances are unloaded, and polling would break the
footprint rule that nothing runs at idle. The user must be able to see and stop everything
that runs on a schedule.

## Decision

- The kernel keeps schedules in its database: owner plugin, key, five-field cron (or
  `@hourly`, `@daily`, `@weekly`, `@monthly`), time zone (the core's local zone or an IANA
  name, through `jiff`), description, enabled, missed-run policy (`skip` or `once`), next
  and last run, last outcome. Manifest triggers become rows owned by the manifest;
  plugins add their own with `kumikit:host/timers`.
- One scheduler task sleeps until the earliest enabled run and is woken by changes. With
  no enabled schedule it arms no timer.
- When a schedule is due the kernel activates the plugin and calls its
  `kumikit:plugin/timers.fire(key, due)` export. A schedule does not fire while its last
  call is still running (recorded as `skipped`).
- Runs missed while the core was down, or by a wake more than 5 minutes late, follow the
  policy: `skip` (the default) records `missed`; `once` runs once. Never more than one
  catch-up.
- Both kinds need `permissions.schedules = true`, shown at install.
- `schedule.list`, `schedule.setEnabled` and `schedule.delete` let any client see, pause
  and remove schedules; disabling a plugin stops its schedules, uninstalling deletes them.
- Cron parsing is Kumikit's own (small, with test vectors); `jiff` is the one new crate.

## Consequences

- Plugins that run on a schedule cost nothing between runs.
- DST: a time skipped by a DST change runs as if the clocks had not moved (02:30 → 03:30);
  a repeated hour runs once.
- `jiff` adds to the core binary and reads the system time-zone database (bundled on
  Windows).
