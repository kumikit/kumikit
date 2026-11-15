# 07 — UI plugins

The UI can be extended in three ways, from cheapest and safest to most powerful.

| Level | Who writes it | Runs code in the webview? | Cost |
|---|---|---|---|
| 1. Declarative contributions | Core plugins and UI plugins | No | None |
| 2. Sandboxed panels (iframe) | UI plugins | Yes, isolated | A few MB per open panel |
| 3. Inline modules | Trusted UI plugins | Yes, in the page | Lowest |

**Core plugins can only use level 1.** Connecting to a remote core therefore never runs
that core's code in your webview ([ADR 0007](../decisions/0007-core-plugins-declarative-ui.md)).

## Slots

| Slot | Content |
|---|---|
| `sidebar.section` | A section in the sidebar |
| `inspector.tab` | A tab in the right-hand inspector |
| `statusbar.item` | A status bar item |
| `composer.accessory` | A control next to the message composer |
| `thread.header` | Controls in a thread's header |
| `transcript.renderer` | A renderer for an event kind, including `plugin.event` kinds |
| `settings.page` | A page in Settings |
| `view` | A full view opened from a command or tab |
| `command` | A palette command |

Slots are versioned with the UI SDK. Adding a slot is a minor change.

## Level 1: declarative contributions

Data in the manifest (or returned by a core plugin at runtime) that the UI kernel renders
with its own components:

- **Settings**: JSON Schema with UI hints (`x-widget`, `x-secret`, `x-order`).
- **Commands**: id, title, icon, keybinding, `when` clause.
- **Status items**: text, icon, tooltip, command; updated by the plugin.
- **Views**: a small vocabulary of models: `list`, `tree`, `table`, `markdown`,
  `key-value`, `form`, `progress`. The plugin supplies data and command ids; the UI kernel
  draws them with the active theme.
- **Notifications and quick picks**.

This covers most plugin UI and keeps it consistent with the theme for free.

## Level 2: sandboxed panels

```toml
id = "community.ui-git-graph"
kind = ["ui"]
api = "^1.0"
[ui]
entry = "dist/index.html"
isolation = "iframe"
[requires]
core = ["kumikit.git@^1"]
[contributes]
slots = [{ slot = "inspector.tab", id = "git-graph", title = "Graph" }]
```

- Loaded into `<iframe sandbox="allow-scripts">` from `kumikit-plugin://<id>/`. CSP:
  no network unless the plugin declares hosts, no top navigation, no popups.
- Talks to the host through a postMessage bridge exposing the UI SDK API asynchronously.
- The host passes the current theme tokens and updates them live, so panels can match.
- Calls to the core go through the bridge and are limited to what the plugin declared
  (`requires.core` methods and read-only kernel methods).

## Level 3: inline modules

```ts
import type { Plugin } from '@kumikit/ui-sdk';

export default {
  activate(ctx) {
    ctx.slots.register('transcript.renderer', {
      kind: 'plugin.event:community.diagram/rendered',
      mount(el, { event }) {
        el.textContent = event.data.title;
        return () => { el.textContent = ''; };   // unmount
      },
    });
  },
} satisfies Plugin;
```

- Runs in the page with access to the DOM it is given. Used for first-party plugins and
  for plugins the user explicitly marks trusted, and for things where an iframe per item
  is too expensive (transcript renderers).
- `mount(el, props) → unmount` keeps the API framework-agnostic: a plugin can use Svelte,
  another framework or none. Svelte plugins compile to plain JS and are the lightest.
- A plugin that throws in `mount` is caught, its slot shows an error placeholder, and it
  is disabled after repeated failures.

## UI SDK

`@kumikit/ui-sdk` exposes the same API at levels 2 and 3:

- `ctx.slots.register(slot, contribution)`
- `ctx.commands.register(id, handler)` / `ctx.commands.run(id, args)`
- `ctx.connection` — the tab's connection: `call(method, params)`, `subscribe(topic, after)`,
  limited by the plugin's declared needs
- `ctx.theme.tokens` and `ctx.theme.onChange(fn)`
- `ctx.storage` — small per-plugin client-side storage
- `ctx.notify(message)`
