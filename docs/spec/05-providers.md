# 05 — Agent providers

A provider plugin adapts one agent (usually an agent CLI) to the kernel's domain model.
The kernel never names a provider: the model picker, approvals and transcript are built
entirely from what providers report.

Users install, enable, configure and remove the plugins that declare providers in the
client's **Settings → Agents** page, not in the general plugin manager
([06](06-client.md#agents)); a provider's settings (`settings.schema`) are shown there, on
its plugin's card.

## Provider ids

A provider id is qualified by the plugin whose manifest declares it:
`<plugin id>/<local id>`, for example `community.provider-gemini/gemini`. Manifests use
the local id; the protocol and the stored `Thread.provider` use the qualified one
([ADR 0011](../decisions/0011-providers-from-plugins.md)).

## Three ways to write a provider

1. **Declarative ACP.** Many agent CLIs speak the
   [Agent Client Protocol](https://agentclientprotocol.com). The preinstalled
   `kumikit.provider-acp` plugin is a provider backend for the key `acp`
   ([04](04-plugin-host.md#delegated-providers)), so an ACP provider can be a manifest with
   no code:

   ```toml
   id = "community.provider-gemini"
   name = "Gemini CLI"
   version = "0.1.0"
   kind = ["provider"]
   api = "^0.2"
   [permissions]
   spawn = ["gemini"]
   [contributes]
   providers = [{ id = "gemini", name = "Gemini CLI", acp = { command = "gemini", args = ["--experimental-acp"] } }]
   ```

   The `acp` table has `command`, `args`, optional `env` (plain variables), `secret-env`
   (variable name → secret name, checked against `permissions.secrets`), `models` (an
   optional static list of `{ id, label, capabilities }`; when omitted, `provider-acp`
   probes the agent's `session/new` response for `configOptions` with `category = "model"`
   or `models.availableModels`, falling back to `default` if the agent reports none or
   cannot be reached), `model-arg` (a flag that passes the chosen model on the agent's
   command line, for example `--model`; when omitted, `provider-acp` selects a non-default
   model through ACP `session/set_config_option` or `session/set_model`), `mode` (a
   session mode to set after opening, for agents whose default mode never asks for
   approval), `auth-method` (an ACP authentication method id to call with `authenticate`
   after `initialize`, for agents that require it before opening a session) and `steer`
   (boolean, default false: opt-in for agents that accept a second `session/prompt`
   mid-turn; adds the `steer` capability to their models). The plugin
   host runs delegated `list-models` probes in a short-lived instance with the declaring
   plugin's granted permissions and an isolated probe directory (`KUMIKIT_PROBE_DIR`), and
   caches results per provider until the TTL expires or a plugin changes. The table is
   checked against the plugin's `acp.schema.json`. The plugin host passes the thread's
   permission mode (`standard`, `accept_edits`, `plan`, `full_access`) in
   `session-config.options` under `"permission-mode"`, and materializes user-message
   attachments into `/data/attachments/` alongside a `manifest.json` referenced from
   `user-input.text`. `provider-acp` maps the permission mode to the agent's advertised
   ACP mode / permissions config option (auto-approving requests in `full_access`, and
   edit requests in `accept_edits` when the agent lacks a native accept-edits mode),
   advertises any other ACP `configOptions` of type `select` (excluding `mode`, `model`
   and `permissions`, mapping category `thought_level` to kind `effort`) as model
   settings via `kumikit:models@0.1.0` and applies chosen values with
   `session/set_config_option` after opening the session,
   sends image attachments as ACP `image` blocks when `promptCapabilities.image` is true
   (or file-path references otherwise), starts the agent in the thread's workspace with
   the contributing plugin's permissions, and translates ACP session updates into kernel
   events: message chunks to `text.delta`/`text.done`, thought chunks to
   `reasoning.delta`, tool calls to `tool.*` (diffs to `file.changed`), permission
   requests to approvals, plans and unknown updates to `plugin.event` (`acp.<update>`),
   and the prompt's token usage and reported cost to `usage`. Session bookkeeping
   (command lists, titles, mode and config changes) and other notifications are
   dropped. It offers the agent no file-system or terminal client capabilities.
   `plugins/provider-acp/examples/claude-code` is a complete declarative provider for
   Claude Code through its ACP adapter. The first-party declarative providers are
   `kumikit.provider-antigravity` (`agy acp`), `kumikit.provider-copilot` (`copilot
   --acp`), `kumikit.provider-cursor` (`cursor-agent acp`) and
   `kumikit.provider-opencode` (`opencode acp`). When the session
   has a tool gateway, `provider-acp` passes it to the agent in `mcpServers`.
2. **WASM adapter.** For agents with their own stream format (for example JSON-lines
   output or an app-server protocol), a plugin implements `kumikit:provider` and spawns
   the CLI through `kumikit:host/process`. `kumikit.provider-claude` runs Claude Code
   this way, over its `stream-json` input and output with permission requests on the same
   stream, and passes the gateway with `--mcp-config`. It is also a provider backend (key
   `claude`: `command`, `args`, `env`, `secret-env`, `models`, `model-arg`, `permission-mode`), so a manifest can run
   Claude Code against another Anthropic-compatible endpoint;
   `providers/claude/examples/deepseek` in `kumikit/kumikit-plugin-agents` does that for DeepSeek.
3. **API provider.** A plugin that calls a model API over `wasi:http` and runs its own
   agent loop, using the tool gateway for tools (when `http` is supported).

## The provider world

```wit
package kumikit:provider@0.2.0;

interface types {
  record model { id: string, label: string, capabilities: list<string> }
  record provider-ref {
    id: string,                        // local id from the manifest that declares it
    plugin: string,                    // the declaring plugin
    config: option<string>,            // backend entry as JSON, for delegated providers
  }
  record session-config {
    thread: string, workspace-root: string, model: string,
    resume: option<string>,            // provider's own session id, if resuming
    mcp-server: option<mcp-server>,    // tool gateway for this session, if granted `tools`
    options: list<tuple<string, string>>,
  }
  record user-input { text: string }
  record mcp-server {                  // a stdio MCP server for the agent to start
    name: string, command: string, args: list<string>,
    env: list<tuple<string, string>>,
  }
}

interface provider {
  use types.{model, provider-ref, session-config, user-input};
  use kumikit:host/types@0.2.0.{error};
  list-models: func(provider: provider-ref) -> result<list<model>, error>;
  start-session: func(provider: provider-ref, config: session-config) -> result<string, error>;
  send: func(session: string, input: user-input) -> result<_, error>;     // runs a turn
  end-session: func(session: string);
}

world plugin {
  import kumikit:host/log@0.2.0;
  import kumikit:host/kv@0.2.0;
  import kumikit:host/events@0.2.0;    // emit text.delta, tool.*, usage, …; request-approval
  import kumikit:host/process@0.2.0;
  import kumikit:host/secrets@0.2.0;
  export kumikit:plugin/lifecycle@0.2.0;
  export provider;
}
```

`send` returns when the turn ends; events stream out through `kumikit:host/events` while
it runs. The kernel emits `turn.started` before the call and `turn.ended` after it, so a
provider cannot leave a turn open.

Each session runs in its own instance of the plugin
([ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md)), so `send` blocking does
not hold up other threads.

- **Approvals.** `events.request-approval(kind, summary, detail)` appends
  `approval.requested` and blocks until the approval is answered; it returns the
  decision, or `cancelled` if the turn was cancelled.
- **Cancel.** The import the plugin is blocked in returns `cancelled` once. The plugin may
  then tell its agent to stop (ACP `session/cancel`) and should return from `send`. The
  kernel ends the turn at once; if `send` has not returned within 2 s, the session's
  processes are killed and its instance is discarded. The next turn starts the session
  again with `resume`.
- After a session's instance is unloaded (idle, failure), the next turn calls
  `start-session` again with `resume` set.

## What the kernel does around a provider

- Persists the provider's session id (`resume`) so threads survive restarts.
- Routes `approval.requested` to every attached client and to bots; the first answer wins
  and is recorded with the deciding device.
- Enforces one running turn per thread; queues further messages in the kernel.
- Kills the provider's processes on cancel (after the grace period), unload or shutdown.

## Steering

Decided in [ADR 0028](../decisions/0028-steering-a-running-turn.md). A provider whose agent
can accept user input during a running turn lists `kumikit:turn/provider@0.1.0` next to
`kumikit:provider/plugin@0.2.0` in `runtime.worlds`, reports `steer` in its models'
capabilities, and imports `kumikit:turn/input@0.1.0`:

- `take() -> result<option<user-input>, error>` takes the oldest waiting steering message
  without blocking.
- `read-stdout(child, max) -> result<read, error>` reads stdout like `child.read-stdout`,
  but returns early with `read.input(user-input)` when a steering message arrives (preferring
  waiting input over stdout so streaming output cannot starve steering).

Whenever a message is taken, the kernel appends `input.steered` to the thread's log. Any
steering messages still in the inbox when `send` finishes are moved to the front of the
queue with `queue.added { front: true, reason: "not_steered" }`. Providers that stay on
`kumikit:provider/plugin@0.2.0` need no changes; `thread.steer` queues their messages at
the front instead.

## Handoff

Decided in [ADR 0029](../decisions/0029-provider-switch-and-handoff.md). When a thread
switches provider (`thread.update { provider, handoff? }`), forks, or loses its stored
session id, the kernel builds a bounded transcript context (at most the newest 200 user
and assistant text messages and 64 KiB encoded, plus `file.changed` paths and an optional
`handoff_summary` note) and renders it as a prompt preamble (`sdk/rust-provider`'s
`handoff::render` shares the exact format). For `kumikit:provider@0.2.0` providers the
kernel prepends that preamble to `user-input.text` while keeping `turn.started` clean,
and appends `handoff.delivered` once the first turn completes `done` with a session id.

## Capabilities

Models report capability strings so the UI adapts without knowing the provider:
`images`, `reasoning`, `tools`, `approvals`, `resume`, `fork`, `plan-mode`, `steer`.
Unknown strings are ignored. New well-known strings are added to this list in the spec.

## Model settings

Decided in [ADR 0033](../decisions/0033-model-settings-and-provider-icons.md). A provider
may attach per-model settings (such as thinking effort or speed) without changing the
published `kumikit:provider@0.2.0` world. The host links the additive import
`kumikit:models/options@0.1.0` for every provider (not listed in `runtime.worlds`):

- `set-options(model, options) -> result<_, error>` attaches a list of settings
  (`id`, `label`, optional `kind` hint — `effort` and `speed` are documented; others are
  free — `values: [{ value, label, description? }]`, and optional `default`) to a model
  returned by the running `list-models` call. Calling it outside `list-models` fails with
  `invalid-argument`.
- The kernel validates thread `options` (`thread.create`, `thread.update`) against the
  model's reported settings, drops values the new model does not offer when the model or
  provider changes, persists them on the thread (inherited by forks and side threads),
  and passes each chosen value to `start-session` in `session-config.options` as
  `("option.<id>", value)`.
- Changing a thread's `options` ends any idle provider session so the next turn resumes
  with the new settings, like a model change.
- A provider entry in `plugin.toml` may also declare `icon = "<path>"` (an `.svg` or
  `.png` inside the package, at most 32 KiB). The plugin host reads it at registration
  and `provider.list` returns it as a `data:` URL in `ProviderInfo.icon`.

## Usage and limits

Decided in [ADR 0035](../decisions/0035-usage-statistics-and-plan-limits.md). Token usage
and cost come from `usage` events, which the kernel adds up for `usage.summary`
([03](03-kernel.md#usage)). Plan limits, such as a 5-hour and a weekly window, come from
`kumikit:usage@0.1.0`, which providers use as far as their agent allows.

**During a turn.** The host links the `report` import for every provider, without a
`runtime.worlds` entry:

- `emit-limits(limits)` reports the account's plan limits as the agent showed them. The
  snapshot is stored for the provider that runs the turn and replaces its earlier one.
  - `limits` is `{ plan?, account?, account-url?, windows }`.
  - Each window is `{ id, label, group?, used-percent (0–100), resets-at? (RFC 3339),
    model? }`.
  - The provider writes the labels ("5 hours", "Weekly"). Clients show them as given.
  - `model` marks a window that covers only some models: a model id, or a family name
    the model ids contain.
- `emit-turn-usage(models)` reports the turn's usage by model, instead of the plain
  `usage` event. Each entry is `{ model, input-tokens (including cached),
  output-tokens, cached-input-tokens, cost? }`. The kernel records one `usage` event
  with the sums and the breakdown. Like `usage`, it is the turn's total so far.
- Outside a turn both fail with `conflict`.

**On demand.** A plugin whose component exports `kumikit:usage/probe` lists
`kumikit:usage/reader@0.1.0` in `runtime.worlds`, next to `kumikit:provider/plugin@0.2.0`
or `kumikit:plugin/plugin@0.2.0`.

- `read-limits(provider-ref)` returns the limits, or `none` when the account has no plan
  limits to show.
- A refresh goes to the plugin that declares the provider, if its component exports the
  reader; otherwise to the provider's backend, if that does.
- So a declarative ACP provider can add a small base-world component only for this, and
  its sessions still run on `provider-acp`:

  ```toml
  [runtime]
  wasm = "reader.wasm"
  worlds = ["kumikit:plugin/plugin@0.2.0", "kumikit:usage/reader@0.1.0"]
  ```

- The host runs `read-limits` in a short-lived instance that is never activated. It gets
  the declaring plugin's grants and its isolated probe directory (`KUMIKIT_PROBE_DIR`),
  and it is cancelled after 30 s.
- It runs only when a user asks (`usage.refresh`), never on a timer. Its errors reach
  the client as `plugin_failed`.
- A reader must not run a model prompt. If the agent's CLI might treat a usage command
  as a prompt, the reader checks the CLI's version first.
- Rust providers use `kumikit_provider::usage` and `export_limits_reader!`.

## Testing providers

- Every provider plugin ships recorded transcripts of its agent's real output (fixtures)
  and a test that replays them through the adapter and compares the emitted events.
- A live smoke test against the installed CLI exists for each provider and is ignored by
  default (`cargo test -- --ignored`).
- Adapters keep the translation from agent output to events free of I/O, so the replay
  test runs natively under `cargo test`; an end-to-end test drives the built component
  with a fake agent that replays the same transcripts.
