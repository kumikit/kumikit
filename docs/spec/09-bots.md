# 09 — Bots

A bot is a core plugin that connects a chat service (Telegram, Slack, Matrix, …) to
threads. It behaves like a headless client: it starts threads, sends messages, relays
events and answers approvals through the same operations the UI uses.

Bots are the test of the protocol's completeness: if a bot cannot do something a user does
in the UI, the plugin API is missing it.

## The bot world

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

`kumikit.bot-telegram` is the first bot.

- **Pairing.** Its command "Telegram: pair a chat" shows a one-time code (8 characters of
  Crockford base32, valid 5 minutes, one use, burnt after 5 wrong tries, one live code).
  The user sends `/pair <code>` to the bot from a private chat; the bot records the chat
  and user id. Group chats are refused. Paired chats are listed on its settings page and
  can be unpaired.
- **Use.** `/workspaces`, `/new <workspace> [provider]`, `/threads`, `/use <n>`, plain
  text (sent to the current thread), `/cancel`. It relays final messages, approval
  requests (Allow and Deny buttons; on by default) and the end of each turn.
- **Limits.** Out: 1 message per second per chat, 25 per second in total, waiting on
  `429`. In: 10 messages per minute per chat; unpaired chats get one "not paired" reply
  per hour.
- **Network.** While enabled with a token set, the core keeps a long poll open to
  `api.telegram.org` (README privacy section). The token is read from the secret store
  when `run` starts and is never logged.

## Webhooks


Bots that need inbound webhooks get them only when the user enables remote access; the
core routes `POST /hooks/<pluginId>/<path>` to the bot's `handle-rpc` export. Long polling
is preferred because it needs no open port.
