# A Kumikit UI plugin in Svelte

A sandboxed panel: it runs in its own frame, reads the tab's core through the UI SDK and
follows the user's theme. Panels need no trust prompt, because they cannot reach the app,
other cores or the network. Copy this folder to start your own.

## What you need

- Node.js 24 or newer, then `npm install` here.
- A Kumikit checkout, until the UI SDK is published to npm:
  `export KUMIKIT_UI_SDK=/path/to/kumikit/web/sdk`.
- The `kumikit` command, to pack and sign.

## Make it yours

1. In `package/plugin.toml`, set `id`, name, description and repository, and choose the
   slot (`inspector.tab`, `sidebar.section`, `view` or `settings.page`).
2. Write the panel in `src/Panel.svelte`. `ctx.connection.call` reads the core
   (`plugin.list`, `thread.list`, `thread.get`, …); `ctx.threads.watch` follows a thread;
   `ctx.storage` keeps small values on this client.
3. If it must contact a web service, list the host in `[ui] network`; users see it.

## Build and sign

```sh
KUMIKIT_KEY=~/.config/kumikit/keys/yourname.key ./build.sh
```

Install the `.kkp` in the app from Manage plugins → On this client → Install from file.
