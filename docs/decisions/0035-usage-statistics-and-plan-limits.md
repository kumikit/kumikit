# 0035 — Usage statistics and plan limits

- **Status:** Accepted
- **Date:** 2026-11-28

## Context

Agent subscriptions have plan limits, such as a 5-hour window and a weekly window. Users
want to see how close the thread's agent is to them before it stops mid-task. They also
want to know what their agents used: tokens, the estimated cost, and which models.

What Kumikit already has, and what it lacks:

- Every turn's `usage` event already records input and output tokens and an optional
  cost in the thread's log. Nothing adds those numbers up across threads, and nothing
  records which models a turn used. One Claude Code turn can run several models, for
  example Haiku subagents inside an Opus turn.
- Providers can say that a limit was *reached* (`limit.reached`, `kumikit:session@0.1.0`),
  but not how much of a limit is *used*.
- How an agent reports limits differs from agent to agent:
  - **Claude Code** sends a `rate_limit_event` during turns: the window (`five_hour`,
    `seven_day`, per-model weekly windows), the share used and the reset time. It also
    answers a `get_usage` control request without running a prompt. That request is
    marked experimental.
  - **ACP** has no limits message. Antigravity's CLI has a separate, prompt-free
    `agy -p /usage --output-format json` report (since 1.1.11). Other ACP agents report
    nothing.
- Published WIT versions can't change in place. The kernel must not name a provider
  ([ADR 0006](0006-kernel-owns-domain-model.md)).

We considered three ways to get limits:

- **Read each vendor's account endpoint with the CLI's saved login.** Rejected: Kumikit
  would read other programs' credentials (macOS stores them in the Keychain), and remote
  cores would need the same login handling.
- **Count usage from each CLI's own local transcripts.** Rejected: the format differs for
  every vendor, and it doesn't work for remote cores.
- **Let providers report what their agent shows** (chosen).

## Decision

- **Statistics come from the kernel's own records.** The kernel writes one row per turn
  (`usage_records`) in the same transaction as each `usage` event. Because `usage` is a
  turn's running total, the row keeps the last report. Rows hold numbers only: provider,
  model, turn start time and token counts. They outlive their thread, so deleting a
  thread doesn't change the statistics. A migration fills the table from existing logs.
  Usage of the same CLI outside Kumikit isn't counted. The UI says so.
- **Protocol 0.12:**
  - `usage.summary { provider?, days, tzOffsetMinutes? }` returns totals, per-model rows
    and per-day tokens (in the client's days) for the turns this core ran.
  - `usage.limits { provider? }` returns each available provider's latest snapshot.
  - `usage.refresh { provider }` asks the provider to read its limits now. Within 60 s of
    the last refresh it returns the stored snapshot instead.
  - A `usage` topic, which keeps no log, carries `usage.limits` and `usage.recorded`.
  - `ProviderInfo.readsLimits` says whether a refresh is possible.
  - `Usage` gains `cachedInputTokens?` and `models?`.
- **`kumikit:usage@0.1.0`**, additive to the provider world:
  - The `report` import, linked for every provider:
    - `emit-limits(limits)`: a snapshot the agent showed during the turn. The kernel
      stores it for the provider that runs the turn.
    - `emit-turn-usage(models)`: the turn's usage by model, used instead of the plain
      `usage` event. The kernel records one `usage` event with the sums and the
      breakdown.
  - The `probe` export, `read-limits(provider-ref)`, in a `reader` world. The manifest
    lists that world next to its main world.
- **Which plugin answers a refresh:**
  - the plugin that declares the provider, if its component exports the reader;
  - otherwise the provider's backend, if that exports it;
  - otherwise the refresh fails with `unsupported`.

  This lets a declarative ACP provider add a small component that only reads limits,
  while its sessions stay on `provider-acp`. The host runs `read-limits` in a short-lived
  instance that is never activated. The instance gets the declaring plugin's grants and
  an isolated probe directory, like a delegated `list-models` probe, and is cancelled
  after 30 s.
- **Snapshots belong to a provider on a core**, not to a thread. They are tidied before
  they are stored: percentages clamped to 0–100, texts cut to 200 bytes, at most 32
  windows, an account URL kept only if it is `https`.
- **Refresh happens only when the user asks.** Nothing polls. Limits seen during turns
  cost nothing extra.
- **A first-party UI plugin, `kumikit.ui-usage`, shows all of this.** It is bundled and
  inline, and uses only the public UI SDK. It adds a meter in the thread header and an
  **Agent usage** dialog with one tab per enabled provider. Labels, groups and model
  scopes come from the provider, so neither the kernel nor `ui-usage` names a provider or
  a window.

## Consequences

- Every provider gets statistics with no work. Providers that can see limits show them
  with no extra request. A refresh runs the agent's own CLI, which contacts its vendor,
  so the README privacy section names it.
- Providers whose agent reports nothing show statistics only.
- The Claude Code adapter depends on stream fields and a control request that Claude Code
  marks experimental. When they change, the meter falls back to the limits seen during
  turns.
- A reader trap counts as a plugin failure, with backoff, like any other call.
- Plugins can't subscribe to the `usage` topic: no permission covers it yet.
- Spec changes:
  - [02-protocol](../spec/02-protocol.md#additions-in-012) and the `usage` topic;
  - [03-kernel](../spec/03-kernel.md#usage);
  - [04-plugin-host](../spec/04-plugin-host.md#interfaces-wit);
  - [05-providers](../spec/05-providers.md#usage-and-limits);
  - [07-ui-plugins](../spec/07-ui-plugins.md);
  - [12-roadmap](../spec/12-roadmap.md#m9--agent-orchestration).
- Tests:
  - the migration's backfill;
  - the kernel's summary rules (a running total counted once, the breakdown by model,
    local days);
  - snapshot checks;
  - the refresh interval and a provider without a reader;
  - through the real core: limits and usage reported by a provider, the reader of a
    provider's own component and of a declarative provider's small component, and the
    `unsupported` and `plugin_failed` paths.
- `kumikit:usage@0.1.0` goes into `wit/PUBLISHED` when it ships in a release.
