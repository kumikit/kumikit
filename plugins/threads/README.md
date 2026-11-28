# kumikit.threads

Agent tools for scheduling wake-ups, inspecting, messaging and starting threads, managing
queues, and suggesting titles via Kumikit's MCP tool gateway.

To get another agent's opinion, an agent can:

1. Call `list_models` to see the providers, their models and each model's settings.
2. Call `start_thread` with a provider, model, settings (`{ "effort": "medium" }`),
   `permissionMode: "plan"` for a read-only thread, and the first message. This needs
   approval.
3. Call `wake_me { afterThread }` to be woken when that thread's turn ends, then
   `read_thread` to read the answer.

A started thread is a side thread of the thread that started it: it shows in that thread's
side panel, not in the sidebar ([ADR 0036](../../docs/decisions/0036-tool-started-threads-are-side-threads.md)).
When `provider` is left out, it uses the calling thread's provider, model and settings, and
its worktree.

A started thread can't be looser than the thread that started it, and never `full_access`
([ADR 0034](../../docs/decisions/0034-agents-start-threads.md)).
