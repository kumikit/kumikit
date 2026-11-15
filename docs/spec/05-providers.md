# 05 — Agent providers

A provider plugin adapts one agent (usually an agent CLI) to the kernel's domain model.
The kernel never names a provider: the model picker, approvals and transcript are built
entirely from what providers report.

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
   api = "^0.1"
   [permissions]
   spawn = ["gemini"]
   [contributes]
   providers = [{ id = "gemini", name = "Gemini CLI", acp = { command = "gemini", args = ["--experimental-acp"] } }]
   ```

   The `acp` table has `command`, `args`, optional `env` (plain variables), `secret-env`
   (variable name → secret name, checked against `permissions.secrets`) and `models` (a
   list of `{ id, label, capabilities }`; default: one model `default` labelled with the
   provider name). `provider-acp` starts the agent in the thread's workspace, with the
   contributing plugin's permissions, and translates ACP session updates into kernel
   events: message chunks to `text.delta`/`text.done`, thought chunks to
   `reasoning.delta`, tool calls to `tool.*` (diffs to `file.changed`), permission
   requests to approvals, and anything else to `plugin.event`. It offers the agent no
   file-system or terminal client capabilities.
2. **WASM adapter.** For agents with their own stream format (for example JSON-lines
   output or an app-server protocol), a plugin implements `kumikit:provider` and spawns
   the CLI through `kumikit:host/process`.
3. **API provider.** A plugin that calls a model API over `wasi:http` and runs its own
   agent loop, using the tool gateway for tools (when `http` is supported).

## The provider world

```wit
package kumikit:provider@0.1.0;

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
    mcp-endpoint: option<string>,      // tool gateway URL for this thread
    options: list<tuple<string, string>>,
  }
  record user-input { text: string }
}

interface provider {
  use types.{model, provider-ref, session-config, user-input};
  use kumikit:host/types@0.1.0.{error};
  list-models: func(provider: provider-ref) -> result<list<model>, error>;
  start-session: func(provider: provider-ref, config: session-config) -> result<string, error>;
  send: func(session: string, input: user-input) -> result<_, error>;     // runs a turn
  end-session: func(session: string);
}

world plugin {
  import kumikit:host/log@0.1.0;
  import kumikit:host/kv@0.1.0;
  import kumikit:host/events@0.1.0;    // emit text.delta, tool.*, usage, …; request-approval
  import kumikit:host/process@0.1.0;
  import kumikit:host/secrets@0.1.0;
  export kumikit:plugin/lifecycle@0.1.0;
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

## Capabilities

Models report capability strings so the UI adapts without knowing the provider:
`images`, `reasoning`, `tools`, `approvals`, `resume`, `fork`, `plan-mode`. Unknown
strings are ignored. New well-known strings are added to this list in the spec.

## Testing providers

- Every provider plugin ships recorded transcripts of its agent's real output (fixtures)
  and a test that replays them through the adapter and compares the emitted events.
- A live smoke test against the installed CLI exists for each provider and is ignored by
  default (`cargo test -- --ignored`).
- Adapters keep the translation from agent output to events free of I/O, so the replay
  test runs natively under `cargo test`; an end-to-end test drives the built component
  with a fake agent that replays the same transcripts.
