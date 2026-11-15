# 05 — Agent providers

A provider plugin adapts one agent (usually an agent CLI) to the kernel's domain model.
The kernel never names a provider: the model picker, approvals and transcript are built
entirely from what providers report.

## Three ways to write a provider

1. **Declarative ACP.** Many agent CLIs speak the
   [Agent Client Protocol](https://agentclientprotocol.com). The preinstalled
   `kumikit.provider-acp` plugin reads `acp` entries from other plugins' manifests, so an
   ACP provider can be a manifest with no code:

   ```toml
   id = "community.provider-gemini"
   kind = ["provider"]
   api = "^1.0"
   [permissions]
   spawn = ["gemini"]
   [contributes]
   providers = [{ id = "gemini", name = "Gemini CLI", acp = { command = "gemini", args = ["--experimental-acp"] } }]
   ```

2. **WASM adapter.** For agents with their own stream format (for example JSON-lines
   output or an app-server protocol), a plugin implements `kumikit:provider` and spawns
   the CLI through `kumikit:host/process`.
3. **API provider.** A plugin that calls a model API over `wasi:http` and runs its own
   agent loop, using the tool gateway for tools.

## The provider world

```wit
package kumikit:provider@1.0.0;

interface types {
  record model { id: string, label: string, capabilities: list<string> }
  record session-config {
    thread: string, workspace-root: string, model: string,
    resume: option<string>,            // provider's own session id, if resuming
    mcp-endpoint: option<string>,      // tool gateway URL for this thread
    options: list<tuple<string, string>>,
  }
  record user-input { text: string, attachments: list<attachment> }
  enum decision { allow, allow-always, deny }
}

interface provider {
  use types.{model, session-config, user-input, decision};
  list-models: func() -> result<list<model>, error>;
  start-session: func(config: session-config) -> result<string, error>;   // session id
  send: func(session: string, input: user-input) -> result<_, error>;     // runs a turn
  respond-approval: func(session: string, approval: string, decision: decision) -> result<_, error>;
  cancel: func(session: string);
  end-session: func(session: string);
}

world plugin {
  import kumikit:host/events;      // emit text.delta, tool.*, approval.requested, usage, …
  import kumikit:host/process;
  import kumikit:host/secrets;
  import kumikit:host/settings;
  export kumikit:plugin/lifecycle;
  export provider;
}
```

`send` returns when the turn ends; events stream out through `kumikit:host/events` while
it runs. The kernel emits `turn.started` before the call and `turn.ended` after it, so a
provider cannot leave a turn open.

## What the kernel does around a provider

- Persists the provider's session id (`resume`) so threads survive restarts.
- Routes `approval.requested` to every attached client and to bots; the first answer wins
  and is recorded with the deciding device.
- Enforces one running turn per thread; queues further messages in the kernel.
- Kills the provider's processes on cancel, unload or shutdown.

## Capabilities

Models report capability strings so the UI adapts without knowing the provider:
`images`, `reasoning`, `tools`, `approvals`, `resume`, `fork`, `plan-mode`. Unknown
strings are ignored. New well-known strings are added to this list in the spec.

## Testing providers

- Every provider plugin ships recorded transcripts of its agent's real output (fixtures)
  and a test that replays them through the adapter and compares the emitted events.
- A live smoke test against the installed CLI exists for each provider and is ignored by
  default (`cargo test -- --ignored`).
