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
| `thread.view` | The main view of a thread; the UI kernel mounts the first enabled one |
| `transcript.renderer` | A renderer for an event kind, including `plugin.event` kinds |
| `settings.page` | A page in Settings |
| `view` | A full view opened from a command or tab |
| `command` | A palette command |

Slots are versioned with the UI SDK. Adding a slot is a minor change.

## Versions

The UI SDK is at **0.1** and freezes at 1.0, before third parties publish (M4)
([ADR 0014](../decisions/0014-ui-sdk-versioning.md)). A UI plugin's or theme's `api` is a
range over the UI SDK version (`api = "^0.1"`), separate from the kernel plugin API of
core plugins. The slot list and the token set of each published SDK version are files in
`web/sdk/api/<version>/` (`slots.json`, `tokens.json`); `web/sdk/api/PUBLISHED` records
their hashes, and a test fails if a published version changes.

## Manifest

UI plugins and themes use the same `plugin.toml` rules as core plugins for `id`, `name`,
`version`, `description`, `license` and `repository`, checked against
`sdk/schema/ui-plugin-manifest.schema.json`. They differ:

| | Core plugin | UI plugin | Theme |
|---|---|---|---|
| `kind` | provider, tool, bot, integration | `["ui"]` | `["theme"]` |
| `api` | kernel plugin API (`^0.1`) | UI SDK (`^0.1`) | UI SDK (`^0.1`) |
| Code | `[runtime]` (WASM) | `[ui] entry`, `isolation` | none |
| Permissions | `[permissions]` | none; `[requires] core`, and trust for third-party inline code | none |
| Contributes | providers, commands, settings, … | `slots`, `commands`, `settings` | `[theme] variants`, `fonts` |
| Installed by | the core (`plugin.install`) | the shell's client plugin store | the shell's client plugin store |

A package is either a core plugin or exactly one of `ui` and `theme`; each installer
refuses the other's packages. `isolation = "inline"` is level 3; `"iframe"` (level 2) is
refused until M4. The shell checks the structure and that the files exist; the UI kernel
checks the contents when it loads a plugin (slot names, token values, font files).
`requires.core` is stored; tabs gate on it from M3.

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
api = "^0.1"
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
- A plugin that throws in `activate` or `mount` is caught and its slot shows an error
  placeholder. After 3 failures in one session the plugin is disabled and the user is told.

## UI SDK

`@kumikit/ui-sdk` exposes the same API at levels 2 and 3:

- `ctx.slots.register(slot, contribution)`
- `ctx.commands.register(id, handler)` / `ctx.commands.run(id, args)`
- `ctx.connection` — the tab's connection: `call(method, params)`, `subscribe(topic, after)`,
  limited by the plugin's declared needs. Subscriptions fan out from the UI kernel's own,
  because a connection has one subscription per topic.
- `ctx.threads.watch(address, fn)` — the UI kernel's folded view of a thread and its
  events; returns a function that stops watching
- `ctx.theme.tokens` and `ctx.theme.onChange(fn)`
- `ctx.storage` — small per-plugin client-side storage
- `ctx.notify(message)`
- `ctx.ui.markdown(el, text)` — renders markdown with the UI kernel's renderer (raw HTML
  shown as text)
