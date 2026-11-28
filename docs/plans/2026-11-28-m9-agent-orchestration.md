# M9 — Agent orchestration

- **Goal:** Let agents coordinate other agents across providers and models, starting with
  agents that start threads with a chosen model and settings
  ([ADR 0034](../decisions/0034-agents-start-threads.md),
  [roadmap M9](../spec/12-roadmap.md#m9--agent-orchestration)).
- **Status:** active (open milestone: features are added as they come)

## Steps

### Feature 1 — Agents start threads (ADR 0034)

- [x] **Step 1.0 — Decision and docs:** ADR 0034; `03-kernel.md` (client interface),
  `04-plugin-host.md` (WIT table), `12-roadmap.md` (M9), `AGENTS.md` status. Verify with
  `sh scripts/check-docs.sh`.
- [x] **Step 1.1 — WIT `kumikit:threads@0.2.0`:** `wit/kumikit-threads@0.2.0/threads.wit`,
  the 0.1.0 functions plus `list-models` and `create-thread` (a `create-request` record).
  0.1.0 stays unchanged and linked.
- [x] **Step 1.2 — Host:** `crates/kumikit-plugin-host/src/runtime/bindings.rs` (a
  `threads_client_v2` module), `runtime/client.rs` (0.2.0 `Host`: the old functions
  delegate to 0.1.0; `list_models`; `create_thread` with the mode rules in a pure,
  unit-tested function), `runtime/mod.rs` (link 0.2.0 for base and tool worlds). Kernel
  unchanged (`thread_create_opts`, `provider_models`).
- [x] **Step 1.3 — Rust SDK:** `sdk/rust/src/lib.rs` `threads` module generated from
  0.2.0 and re-exporting `list_models`, `create_thread` and `CreateRequest`;
  `sdk/rust-tool` re-exports unchanged.
- [x] **Step 1.4 — Tests through the real core:** `test-plugins/client` gains `models` and
  `create-with` commands; `crates/kumikit-core/tests/integration/client_interface.rs`
  covers the probe's effort setting, invalid settings (`invalid_params`), `full_access`
  refused, looser-than-origin refused, the default clamped to a `plan` origin, an origin
  in another workspace refused, and both new commands denied without `threads.create`.
- [x] **Step 1.5 — `kumikit.threads` tools:** `plugins/threads` gains `list_models`
  (`read`) and `start_thread` (`write`) with schemas, the `threads.create` grant, input
  parsing in `format.rs` with unit tests, an updated README, and plugin version 0.2.0.
- [x] **Step 1.6 — Verify:** fmt, clippy (host and wasm), `cargo test --workspace`,
  `check-docs.sh`, `check-plugin-names.sh`, `check-no-keys.sh`.
- [x] **Step 1.7 — Publish:** add `kumikit-threads@0.2.0` to `wit/PUBLISHED` when it ships
  in a release.

### Feature 2 — Usage statistics and plan limits (ADR 0035)

**What the user sees.** A thread's header (the `thread.header` slot, next to the model
tag) shows a small meter for the thread's provider: the window that is closest to its
limit, for example `5h ▮▮▯▯ 26%`. Its tooltip lists every window and when it resets. A
click opens the **Agent usage** dialog, open on that provider's tab:

- Tabs: **Overview**, then one tab per provider that is installed and enabled on the
  thread's core (from `ProviderInfo`, with its icon). Providers that are disabled or not
  installed don't appear.
- A provider tab has these sections:
  - A header with the icon, the name and "Checked <time>". The info tooltip says where
    the numbers come from.
  - The account and plan, when the provider reports them.
  - One bar per limit window: label, percent used and "Resets …". The provider names the
    windows ("5 hours", "Weekly", per-model weekly windows).
  - **Past 30 days:** estimated cost, tokens, cached input, sessions (threads), and one
    row per model with cost and tokens.
  - **Activity:** a heatmap of tokens per day over the past 20 weeks, with active days
    and a total. Hovering or focusing a day shows that day's tokens.
  - A refresh button, and "Updated on demand" at the bottom.
- **Overview** shows one row per provider: its tightest window and its 30-day cost and
  tokens.
- If a provider reports no limits, its tab shows statistics only and says "This
  provider doesn't report plan limits".

**Where the data comes from.**

- *Statistics* come from the core's own log: the `usage` events of turns that Kumikit
  ran. Usage of the same CLI outside Kumikit isn't counted. The UI says so. The data
  works the same for every provider and for remote cores.
- *Limits* come from the provider, through a new WIT package `kumikit:usage@0.1.0`, in
  two ways:
  - **Pushed during a turn** (`emit-limits`): the provider reports a snapshot whenever
    the agent shows one. This costs nothing extra. Claude Code sends a `rate_limit_event`
    with `rateLimitType`, `utilization` and `resetsAt`.
  - **Read when the user asks** (an exported `read-limits`): the host calls it in a
    short-lived instance, like the `list-models` probes. Claude Code answers its `get_usage`
    control request without running a prompt. That request is experimental in Claude
    Code, so we must handle it disappearing or changing shape. Then the meter falls back
    to the pushed snapshots.
  - The reader doesn't have to live in the session code. ACP has no limits message, so
    a declarative ACP provider can't push limits. Its plugin can instead add a small WASM
    module that only exports `read-limits`. Antigravity works this way: it reads limits
    with a separate, prompt-free `agy -p /usage --output-format json` run, not through
    its ACP session.
- The kernel keeps the latest snapshot for each provider (limits belong to the account,
  not to a thread) and sends changes to clients.

Steps:

- [x] **Step 2.0 — Decision and docs:** ADR 0035, covering:
  - statistics are kernel-derived and count only turns Kumikit ran;
  - limits are reported by providers through `kumikit:usage@0.1.0`, pushed during turns
    and read on demand;
  - snapshots are per provider and per core;
  - refresh happens only when the user asks.

  Update the specs:
  - `02-protocol.md`: protocol 0.12, the `usage` topic.
  - `03-kernel.md`: usage records and limit snapshots.
  - `04-plugin-host.md`: the WIT table and the probe.
  - `05-providers.md`: a "Usage and limits" section.
  - `07-ui-plugins.md`: the bundled `ui-usage`.
  - `12-roadmap.md`: the M9 bullet.
  - The README privacy section: a refresh runs the agent's CLI, which contacts its vendor.

  Verify with `sh scripts/check-docs.sh`.
- [x] **Step 2.1 — WIT `kumikit:usage@0.1.0`:** `wit/kumikit-usage@0.1.0/usage.wit`, with:
  - An imported `report` interface:
    - `limit-window { id, label, group?, used-percent: f32 (0–100), resets-at?, model? }`
      (`model` is set when the window covers only some models);
    - `limits { plan?, account?, account-url?, windows }`;
    - `model-usage { model, input-tokens, output-tokens, cached-input-tokens, cost? }`;
    - `emit-limits(limits)`;
    - `emit-turn-usage(list<model-usage>)`: the per-model breakdown of the running turn,
      for example Haiku subagents inside an Opus turn.
  - An exported `probe` interface: `read-limits(provider-ref) -> result<option<limits>, error>`.
  - Two worlds:
    - `reporter` (import only): used by session code during turns.
    - `reader` (probe export only, with log and process imports): used by any plugin
      that declares the provider, including a declarative ACP provider's plugin with a
      small WASM module.

    A WASM provider such as Claude Code links both.

  The emits are valid only inside a session's turn; anywhere else they return
  `invalid-argument`.
- [x] **Step 2.2 — Protocol 0.12** (`crates/kumikit-protocol`, regenerate `web/protocol/`,
  add fixtures):
  - `Usage` gains `cachedInputTokens?` and `models?`. Each entry is `{ model, inputTokens,
    outputTokens, cachedInputTokens?, cost? }`.
  - `ProviderInfo` gains `usage? { limits, refresh }`, taken from the worlds its plugin
    declares.
  - New methods:
    - `usage.limits { provider? }` returns `[{ provider, limits?, observedAt? }]`.
    - `usage.refresh { provider }` returns a snapshot. A provider without the probe gives
      `unsupported`. A refresh within 60 s of the last one returns the stored snapshot.
    - `usage.summary { provider?, days, tzOffsetMinutes }` returns:
      - `totals { inputTokens, outputTokens, cachedInputTokens, cost?, turns, threads }`;
      - `models` (one row per provider and model);
      - `daily` (`{ date, tokens, cost? }`, local days);
      - `activeDays`.
  - A topic `usage`, which keeps no log (like `workspace:<id>`). Its events are
    `usage.limits` (a new snapshot) and `usage.recorded` (a turn added usage).
- [x] **Step 2.3 — Kernel** (`crates/kumikit-kernel`):
  - A migration adds two tables:
    - `turn_usage`: one row per finished turn and model, with `thread`, `provider`,
      `model` and `ended`. It's derived when `turn.ended` is written, in the same
      transaction, from the turn's **last** `usage` event, because `usage` reports a
      turn's running total. Without `models`, the row uses the turn's model.
    - `provider_limits`: provider, snapshot JSON, observed time.
  - The migration backfills `turn_usage` from the existing logs.
  - Add the summary query, the snapshot store and the `usage` topic. Reads run on
    `spawn_blocking`.
  - Unit tests: cumulative usage counted once, multi-model turns, local-day grouping,
    backfill, and a cancelled turn's partial usage still counted.
- [x] **Step 2.4 — Host** (`crates/kumikit-plugin-host`):
  - Bindings for `kumikit:usage@0.1.0`, and links for the provider world.
  - `emit-limits` stores a snapshot and announces it; `emit-turn-usage` attaches `models`
    to the turn's `usage` event.
  - `read-limits` runs in a short-lived probe instance with the plugin's granted
    permissions (as in `list-models`), under the call time limit.
  - The manifest accepts the new worlds only for `provider` plugins.
  - **Routing.** `read-limits` goes to the plugin that declares the provider when that
    plugin exports `reader`; otherwise to its provider backend (such as `provider-acp`),
    if the backend exports it; otherwise the refresh gives `unsupported`.
  - **WASM alongside a delegated provider.** A plugin with delegated (`acp = …`)
    providers may also ship a `[runtime] wasm` that links only the `reader` world. It's
    instantiated only for `read-limits`, never for sessions. Check what manifest
    validation and activation allow today, and change both if needed.
  - Tests:
    - emit outside a turn is refused;
    - refresh of a provider without the probe gives `unsupported`;
    - a probe timeout gives `unavailable`;
    - the 60 s refresh throttle;
    - a reader in the declaring plugin is used for a delegated provider.
- [x] **Step 2.5 — Test provider and integration tests:** `test-plugins/probe` emits
  limits and per-model usage and implements `read-limits`. Add
  `crates/kumikit-core/tests/integration/usage.rs` with these cases:
  - `usage.summary` after two turns;
  - a `usage.limits` event reaches a `usage` subscriber;
  - `usage.refresh` works;
  - `ProviderInfo.usage` flags are set;
  - disabled providers are left out.
- [x] **Step 2.6 — Claude Code provider** (`kumikit-plugin-agents`, `providers/claude`):
  - Map every `rate_limit_event` (not only `rejected`) into `emit-limits`:
    - `five_hour` becomes "5 hours" and `seven_day` becomes "Weekly";
    - `seven_day_opus` and `seven_day_sonnet` become per-model windows;
    - `utilization` (0–1) becomes a percent.
  - Map the result's `modelUsage` into `emit-turn-usage` (including cache tokens).
  - Implement `read-limits`: start `claude` in stream-json mode, send the `get_usage`
    control request with `skip_behaviors: true`, read `rate_limits` and
    `subscription_type`, then end the process. No prompt is sent.
  - Add recorded replays for each event shape, and one ignored live test.
  - Bump the plugin's minor version and declare the new world.
  - Fallback if `get_usage` goes away: `claude -p /usage --output-format json` with all
    tools off and `--max-turns 1`. Pandacode checked that this makes no model call,
    prints the usage lines and reports `total_cost_usd` 0.
- [x] **Step 2.6b — Antigravity limits reader** (`kumikit-plugin-agents`,
  `providers/antigravity`): a small WASM module linking only the `reader` world, added to
  the declarative manifest. The ACP session is unchanged. `read-limits` does this:
  1. Runs `agy --version` (3 s limit). It goes on only for a plain `X.Y.Z` of at least
     1.1.11. An older `agy` may treat `/usage` as a model prompt, so prereleases and
     unclear output stop here.
  2. Runs `agy -p /usage --output-format json --print-timeout 12s` in an empty temporary
     directory, with `AGY_CLI_DISABLE_AUTO_UPDATE=true` and an output size limit. Stderr
     can hold login details, so it is dropped.
  3. Accepts the report only if `status` is `SUCCESS`, `command.name` is `usage`, and
     both `num_turns` and `usage.total_tokens` are 0.
  4. Maps `command.data.groups[].buckets[]` to windows. The label is `<group name> ·
     5 hours / Weekly / Monthly`, taken from `window` or `id` (`5h`, `gemini-5h`,
     `3p-5h`, …). Used percent is `(1 − remaining_fraction) × 100`; the reset is
     `reset_time`. Disabled buckets and buckets without a fraction are skipped.
  5. Returns no limits when the provider's `env` sets `ANTIGRAVITY_OAUTH_CREDENTIALS_JSON`
     or a custom `GEMINI_DIR`/`ANTIGRAVITY_GEMINI_DIR`. The report can't target that
     account, so it would show another account's limits.

  Tests: a replay of the report fixture, the version gate, refusal of a report from a
  turn that ran a prompt, and one ignored live test. Antigravity never pushes limits
  during turns.
- [x] **Step 2.7 — `ui-usage` UI plugin** (`plugins/ui-usage`, bundled through the data
  list in `scripts/bundle.mjs`, inline, public UI SDK only):
  - **`thread.header` meter.** A button with `role="meter"`-style labelling and the
    tightest window that applies to the thread's model. It uses warning and danger tokens
    at 80% and 95%, and shows `limited` when the thread is limited. It stays hidden when
    the provider reports no limits.
  - **Dialog.** A native `<dialog>` like `ChangesDialog`: tabs, provider sections, a
    per-model table and the heatmap (a grid of labelled cells; arrow keys move between
    days).
  - Only theme tokens; reduced motion respected; Escape closes; focus comes back to the
    meter.
  - Subscribes to `usage` for live updates.
  - Vitest:
    - picking the tightest window;
    - formatting percents, resets and tokens (`1.6B`, `31.9M`);
    - heatmap buckets;
    - tab list from enabled providers only.
- [x] **Step 2.8 — End to end:** a browser e2e case with the test provider: meter
  appears, tooltip, dialog opens on the right tab, refresh updates the bar, axe passes in
  light and dark. Check the bundle size budget.
- [x] **Step 2.9 — Verify:** fmt, clippy (host and wasm), `cargo test --workspace`,
  `npm run check`, `npm test`, the e2e suite, `check-docs.sh`, `check-plugin-names.sh`,
  `check-no-keys.sh`, `footprint.sh` (no instance at startup: the probe runs only on
  refresh).
- [x] **Step 2.10 — Publish:** add `kumikit-usage@0.1.0` to `wit/PUBLISHED` when it
  ships in a release.

### Feature 3 — Started threads are side threads (ADR 0036)

**What the user sees.** A thread that an agent starts with `start_thread` appears in the
**Side thread** panel of the thread that started it, not in the sidebar. The panel is the
inspector tab that already lists the side threads (`/btw`) of the active thread.

- [x] **Step 3.0 — Decision and docs:** ADR 0036; ADR 0034 status note; `03-kernel.md`
  (client interface); the threads README and tool description; the Notes above.
- [x] **Step 3.1 — Host:** `runtime/client.rs` `create_thread` passes the origin as
  `parent` and sets `side`. The permission mode is always passed when there is an origin,
  so the kernel never inherits a looser mode from the origin.
- [x] **Step 3.2 — Tests:** `client_interface.rs` checks that a thread with an origin has
  `parent` and `side`, is hidden from `thread.list` and shown with `includeSide`, that a
  thread without an origin has neither, and that an `accept_edits` origin gives `standard`
  when no mode is asked for.
- [ ] **Step 3.3 — Verify the side panel:** a started thread shows in the side panel of its
  origin, live, and not in the sidebar. Checked by reading `plugins/ui-chat/src/SideThread.svelte`
  (it loads `thread.list` with `includeSide` and keeps threads whose `parent` is the
  active thread, live from `thread.created`); not yet run in the desktop app or e2e.
- [ ] **Step 3.4 — Verify:** fmt, clippy (host and wasm), `cargo test --workspace`,
  `check-docs.sh`, `check-plugin-names.sh`, `check-no-keys.sh`.

## Notes

- Feature 2: the kernel and `ui-usage` never name a provider or a window. Labels, groups
  and model scopes come from the provider. "5 hours" lives in the Claude adapter, not in
  Kumikit.
- Feature 2, prior art: pandacode (`../pandacode`, `docs/agent-usage.md`,
  `src-tauri/src/agent_usage_providers.rs`) reads each agent's limits outside its
  session. For Antigravity it runs the version-gated `agy -p /usage` report; it can't do
  this for installations that have only the ACP server. For Claude Code it reads Claude
  Code's saved login and calls its OAuth usage endpoint. Kumikit doesn't read
  credentials. The provider's own CLI reads them (`get_usage`, `/usage`).
- Feature 2: the account (for example an email) is shown only if the provider reports
  it. It stays on the core and its clients and is never logged.
- Feature 1: the protocol doesn't change: `thread.create` already takes `options` and
  `permissionMode` (protocol 0.11).
- `origin` is used for checks (same workspace, mode bound) and as the `parent` of the
  started thread: started threads are side threads of the calling thread
  ([ADR 0036](../decisions/0036-tool-started-threads-are-side-threads.md), Feature 3).
- `kumikit.threads` 0.2.0 widens its grant (`threads.create`), so a bundled upgrade waits,
  disabled, for the user to grant it again.
- Through `command.run`, a plugin's `invalid-argument` reaches clients as
  `invalid_params`.
