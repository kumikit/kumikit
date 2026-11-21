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

The UI SDK is at **0.1.2** (0.1.0 plus sandboxed panels in 0.1.1, and `CommandResult`
and `x-secret` secret names in 0.1.2) and freezes at 1.0 after M5,
before the official registry opens to third parties
([ADR 0014](../decisions/0014-ui-sdk-versioning.md),
[ADR 0020](../decisions/0020-interface-freeze-after-m5.md)). A UI plugin's or theme's `api` is a
range over the UI SDK version (`api = "^0.1"`), separate from the kernel plugin API of
core plugins. The slot list and the token set of each published SDK version are files in
`web/sdk/api/<version>/` (`slots.json`, `tokens.json`); `web/sdk/api/PUBLISHED` records
their hashes, and a test fails if a published version changes. A version that changes neither list
(0.1.1, 0.1.2) adds no directory; its lists are those of 0.1.0.

## Manifest

UI plugins and themes use the same `plugin.toml` rules as core plugins for `id`, `name`,
`version`, `description`, `license` and `repository`, checked against
`sdk/schema/ui-plugin-manifest.schema.json`. They differ:

| | Core plugin | UI plugin | Theme |
|---|---|---|---|
| `kind` | provider, tool, bot, integration | `["ui"]` | `["theme"]` |
| `api` | kernel plugin API (`^0.1`) | UI SDK (`^0.1`) | UI SDK (`^0.1`) |
| Code | `[runtime]` (WASM) | `[ui] entry`, `isolation` | none |
| Permissions | `[permissions]` | none; `[requires] core`, `[ui] network` for panels, and trust for third-party inline code | none |
| Contributes | providers, commands, settings, … | `slots`, `commands`, `settings` | `[theme] variants`, `fonts` |
| Installed by | the core (`plugin.install`) | the shell's client plugin store | the shell's client plugin store |

A package is either a core plugin or exactly one of `ui` and `theme`; each installer
refuses the other's packages. `isolation = "inline"` is level 3; `"iframe"` is level 2,
whose `entry` is an HTML file. The shell checks the structure and that the files exist; the UI kernel
checks the contents when it loads a plugin (slot names, token values, font files).
`requires.core` lists core plugins as `<plugin id>@<semver range>` (for example
`kumikit.git@^1`). An entry is met on a connection when that core reports the plugin as
enabled (in `core.hello` or the latest `core.pluginsChanged`) at a version in the range. A
UI plugin's module is loaded and activated once; its slot contributions and commands are
offered only while the active tab's core meets every entry, and `thread.view` and
`transcript.renderer` are chosen per tab. A plugin without `requires.core` shows on every
core.

## Level 1: declarative contributions

Data in the manifest (or returned by a core plugin at runtime) that the UI kernel renders
with its own components:

- **Settings**: JSON Schema with UI hints (`x-widget`, `x-secret`, `x-order`).
- **Commands**: id, title, icon, keybinding, `when` clause. A core plugin's commands
  (`command.list`) appear in the palette for tabs on that core; running one
  (`command.run`, in the tab's workspace for `workspace` commands) shows its result: a
  notification for `message`, and a view for `view`, whose `form` submits by running the
  command it names with the form's values.
- **Status items**: text, icon, tooltip, command; updated by the plugin.
- **Views**: a small vocabulary of models: `list`, `tree`, `table`, `markdown`,
  `key-value`, `form`, `progress`. The plugin supplies data and command ids; the UI kernel
  draws them with the active theme.
- **Notifications and quick picks**.

The vocabulary's types (`DeclarativeView`, `SettingsSchema`, `StatusItem`,
`CommandResult`) are part of the UI SDK (`CommandResult` since 0.1.2). Settings pages for
core plugins are drawn from `settings.schema`; a field with `x-secret` shows only "set" or
"not set" and writes with `secret.set`. A UI plugin's `contributes.settings` names a schema file; the UI kernel draws
its page and keeps the values in the plugin's client storage.

This covers most plugin UI and keeps it consistent with the theme for free.

## Level 2: sandboxed panels

```toml
id = "community.ui-git-graph"
kind = ["ui"]
api = "^0.1.1"
[ui]
entry = "dist/index.html"
isolation = "iframe"
network = []                       # https hosts the panel may contact
[requires]
core = ["kumikit.git@^1"]
[contributes]
slots = [{ slot = "inspector.tab", id = "git-graph", title = "Graph" }]
```

Decided in [ADR 0019](../decisions/0019-sandboxed-panels.md).

- **Slots.** A panel contributes through `[contributes] slots` to `sidebar.section`,
  `inspector.tab`, `view` or `settings.page`; other slots are refused for `iframe`
  plugins. Its document is loaded only when its slot is shown.
- **Frame.** `<iframe sandbox="allow-scripts">` loading `[ui] entry` from the plugin's own
  URL (`kumikit-plugin://<id>/`, `http://kumikit-plugin.localhost/<id>/` on WebView2,
  `/ui-plugins/<id>/` in the browser client), with no referrer. Without
  `allow-same-origin` the panel has an opaque origin: it cannot reach the app's DOM,
  storage or cookies. It cannot navigate the top window, open popups, submit forms or
  show modal dialogs. If the frame loads a second document, the UI kernel removes it,
  closes its bridge and says so in the slot.
- **CSP.** Every file of a panel plugin is served with its own CSP: `default-src 'none'`;
  scripts, styles, fonts and images (and `data:` images) only from that plugin's
  directory; `connect-src` only the `https` hosts listed in `[ui] network` (none by
  default); no frames, forms, base changes or objects. The panel's files are sent with
  `Access-Control-Allow-Origin: null`, because its opaque origin fetches its module
  scripts with `Origin: null`. In the browser client they may be framed only by the
  core's own page (`frame-ancestors` the core's origin); every other file keeps
  `frame-ancestors 'none'`.
- **Network.** `[ui] network = ["api.example.com"]` lists the hosts the panel may contact.
  They are shown at install and when an update adds one.
- **Bridge.** At the frame's first load the UI kernel transfers a `MessagePort` to it
  (`{ type: "kumikit.panel", version: 1 }`); all later messages use that port, and other
  `window` messages are ignored. Calls are `{ id, method, params }` answered by
  `{ id, result }` or `{ id, error }`, at most 1 MiB each and 64 in flight.
- **SDK.** `@kumikit/ui-sdk/panel` exports `connectPanel(): Promise<PanelContext>`: the
  UI SDK below, asynchronous: `slot` and `props` (the tab's address), `connection.call` and
  `connection.subscribe`, `threads.watch`, `theme` (tokens, appearance, `onChange`; the SDK
  applies the tokens to the panel's document as `--kk-*` properties and the UI kernel
  pushes changes live), `storage` (async `get`, `set`, `delete`), `notify`, and
  `commands.run` for the plugin's own commands.
- **What a panel may call.** Only its tab's connection, and only:
  - read-only kernel methods: `workspace.list`, `thread.list`, `thread.get`,
    `provider.list`, `provider.models`, `plugin.list`, and subscriptions to `thread:` and
    `workspace:` topics, with thread and workspace reads limited to the tab's workspace;
  - `<pluginId>/*` methods of the plugins in its `requires.core`; their `workspace`
    param is always the tab's (the bridge replaces whatever the panel sends).

  Everything else fails with `permission_denied`: other connections, writes (`thread.send`,
  approvals, `plugin.*`, `registry.*`, `settings.*`, `secret.*`, `auth.*`,
  `core.shutdown`), and every shell command (profiles, the client plugin store). The
  shell's IPC is not reachable from a panel.
- **Trust.** Panels need no trust prompt; changing a plugin from `iframe` to `inline` in an
  update asks for trust again.

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
- `ctx.theme.tokens`, `ctx.theme.appearance` and `ctx.theme.onChange(fn)`; `ctx.theme.variants`,
  `ctx.theme.choice` and `ctx.theme.select(choice)` list and switch the installed themes
- `ctx.storage` — small per-plugin client-side storage
- `ctx.notify(message)`
- `ctx.ui.markdown(el, text)` — renders markdown with the UI kernel's renderer (raw HTML
  shown as text)
- `ctx.ui.renderItem(el, props)` — mounts the `transcript.renderer` registered for an item
  (by any plugin), or a plain fallback, so a thread view shows other plugins' renderers
