# 09 — Bots

A bot is a core plugin that connects a chat service (Telegram, Slack, Matrix, …) to
threads. It behaves like a headless client: it starts threads, sends messages, relays
events and answers approvals through the same operations the UI uses.

Bots are the test of the protocol's completeness: if a bot cannot do something a user does
in the UI, the plugin API is missing it.

## The bot world

```wit
package kumikit:bot@1.0.0;

interface client {
  // The protocol surface, scoped to what the bot was granted.
  list-workspaces: func() -> result<list<workspace>, error>;
  create-thread: func(workspace: string, provider: string, model: string) -> result<string, error>;
  send: func(thread: string, text: string) -> result<_, error>;
  respond-approval: func(approval: string, decision: decision) -> result<_, error>;
  subscribe: func(topic: string, after: u64) -> result<subscription, error>;
}

interface bot {
  run: func() -> result<_, error>;   // long-running; returns when stopped
}

world plugin {
  import client;
  import kumikit:host/secrets;
  import kumikit:host/kv;
  import wasi:http/outgoing-handler@0.2.0;
  export kumikit:plugin/lifecycle;
  export bot;
}
```

`run` is activated on `startup` when the bot is enabled. It long-polls or holds a
WebSocket to the chat service through `wasi:http` and reads kernel events through its
subscription.

## Permissions

```toml
[permissions]
network = ["api.telegram.org"]
secrets = ["telegram-bot-token"]
client = ["threads.create", "threads.send", "approvals.respond"]
workspaces = "chosen"      # only the workspaces the user links to this bot
```

- `client` lists the protocol operations the bot may perform; the kernel enforces them.
- Approvals answered by a bot are recorded with the bot as the deciding party.
- Who may talk to the bot (chat IDs, user IDs) is a bot setting, enforced by the bot and
  shown in its settings page. The kernel records every message a bot sends into a thread.

## Webhooks

Bots that need inbound webhooks get them only when the user enables remote access; the
core routes `POST /hooks/<pluginId>/<path>` to the bot's `handle-rpc` export. Long polling
is preferred because it needs no open port.
