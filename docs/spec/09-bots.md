# 09 — Bots

A bot is a core plugin that connects a chat service (Telegram, Slack, Matrix, …) to
threads. It behaves like a headless client: it starts threads, sends messages, relays
events and answers approvals through the same operations the UI uses.

Bots are the test of the protocol's completeness: if a bot cannot do something a user does
in the UI, the plugin API is missing it.

## The bot world (planned)

```wit
package kumikit:bot@0.1.0;

interface bot {
  use kumikit:host/types@0.2.0.{error};
  run: func() -> result<_, error>;   // long-running; returns when stopped
}

world plugin {
  import kumikit:host/log@0.2.0;
  import kumikit:host/kv@0.2.0;
  import kumikit:host/secrets@0.2.0;
  import kumikit:host/settings@0.2.0;
  import kumikit:host/client@0.2.0;   // the protocol surface, within the bot's grant
  import wasi:http/outgoing-handler@0.2.0;
  export kumikit:plugin/lifecycle@0.2.0;
  export kumikit:plugin/commands@0.2.0;
  export bot;
}
```

`run` starts on its own instance when the bot is enabled (`activation.on = ["startup"]`)
and restarts with the host's backoff if it fails. It long-polls the chat service through
`wasi:http` ([04](04-plugin-host.md#outgoing-http)) and waits on that request and its
kernel subscriptions at once (`wasi:io/poll`). The client interface
([03](03-kernel.md#client-interface)) is shared with other plugins that act as clients.

## Permissions

```toml
[permissions]
network = ["api.telegram.org"]
secrets = ["telegram-bot-token"]
client = ["workspaces.list", "threads.read", "threads.create", "threads.send", "threads.cancel", "approvals.respond"]
workspaces = "linked"      # only the workspaces the user links to this bot
```

- `client` lists the operations the bot may perform; the kernel enforces them.
- Approvals answered by a bot are recorded with the bot as the deciding party
  (`approval.resolved.plugin`); a bot cannot answer `allow_always`.
- Who may talk to the bot (chat IDs, user IDs) is a bot setting, enforced by the bot and
  shown in its settings page. The kernel records every message a bot sends into a thread
  (`turn.started.from`).

## Telegram

A Telegram bot is planned for a milestone after M5. The client interface it needs
(`kumikit:host/client`, linked workspaces, `from` and `plugin` in the log) is built; the bot
world above is not yet.

## Webhooks


Bots that need inbound webhooks get them only when the user enables remote access; the
core routes `POST /hooks/<pluginId>/<path>` to the bot's `handle-rpc` export. Long polling
is preferred because it needs no open port.
