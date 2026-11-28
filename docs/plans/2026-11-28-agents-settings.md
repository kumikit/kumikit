# Plan — An Agents page in Settings

Agent providers (Claude Code, Copilot, Cursor, OpenCode, Antigravity, …) move out of the
general plugin manager into their own **Settings → Agents** page. There the user sees the
agents installed on a core and the agents its registries offer, installs, enables,
disables, updates and removes them, and edits each agent's settings in place. The Plugins
view and its registry search stop showing agents.

## What counts as an agent

A core plugin is an **agent** when its manifest declares at least one provider
(`[[contributes.providers]]`). A registry result is an agent when its `kind` includes
`provider`. A plugin that only adds a provider backend, such as the bundled
`kumikit.provider-acp`, is not an agent: it has nothing to pick in the model picker, so it
stays in Plugins.

Today the client cannot tell this for a disabled plugin: `InstalledPlugin.contributes.providers`
lists only providers that are available now, which is empty while the plugin is disabled.
So the protocol gains the declared providers.

## Steps

- [x] **1. Protocol 0.13.** Add the optional `InstalledPlugin.providers`: the providers the
  manifest declares, enabled or not, as `{ id (qualified), name, icon? }` (the same `icon`
  `data:` URL as `provider.list`). A minor change. Add it to a recorded fixture, regenerate
  `web/protocol/`, bump the handshake version, and update `02-protocol.md` ("Additions in
  0.13", the `plugin.list` row).
- [x] **2. Core.** Fill `providers` from the manifest in `crates/kumikit-core/src/plugins.rs`
  for `plugin.list` and the install, enable and update results. Integration test: a
  disabled provider plugin still lists its declared provider with its name and icon.
- [x] **3. Shared plugin pieces in the UI kernel.** Move the grant, remove and update
  dialogs out of `PluginsView.svelte` into `CorePluginDialog.svelte`, which both views use,
  so the permission wording and signing warnings stay in one place. The cards stay apart:
  an agent's card (icons, settings, update) differs from a plugin's.
- [x] **4. Settings → Agents.** Add a kernel-owned `AgentsSettings.svelte` page, listed
  first in the core group of the Settings navigation. It is part of the UI kernel, so it
  also shows in safe mode. For the chosen core (the active tab's by default, with the
  same core selector as Plugins):
  - **Installed:** one card per agent with its icon and provider names, version, state and
    signing; **Enable…** (the grant dialog), **Disable**, **Update** when the registry has
    a newer version, and **Remove…**. An enabled agent with settings shows them in the card,
    in an expandable **Settings** section using the existing `CoreSettings` form. Problems
    (`failure`, `error`) show as in Plugins.
  - **Available:** the registries' agents not yet installed (`registry.search`, filtered on
    `kind`), with name, publisher, description, permissions and **Install**, plus a search
    box for this list only.
  - **Install from file…** (and from a folder on the local core), as in Plugins.
- [x] **5. Take agents out of the rest.** In Plugins, the "On this core" list, registry
  search results and the Updates list leave out agents, with a one-line link "Agents are
  managed in Settings → Agents". The **starter pack** keeps its agents, since setting one
  up is the point of first run. The Settings "On <core>" group leaves out agents, whose
  settings now live on the Agents page. The palette gets an **Agents** command, and the new
  thread composer's "no providers" prompt links to the Agents page instead of Plugins.
- [x] **6. Tests.** Vitest: agent classification (installed and registry), the Agents page
  (lists, install from registry with the grant dialog, enable, disable, remove, update,
  settings shown only when enabled), and the filters in Plugins and Settings. Playwright:
  install an agent from the fixture registry on the Agents page, change one of its
  settings, see it absent from the Plugins search, remove it; axe in light and dark.
- [x] **7. Spec and docs.** `06-client.md` (Plugin manager, Settings: the Agents page and
  what counts as an agent), `05-providers.md` (where users manage providers),
  `10-distribution.md` (starter pack note), README if it describes installing agents.
- [x] **8. Verify.** `check-docs.sh`, `check-plugin-names.sh`, `check-no-keys.sh`,
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo test -p kumikit-protocol`, `tsc --noEmit -p
  web/protocol`, `npm run check`, `npm test`, `node scripts/bundle.mjs`, and the Playwright
  Chromium suite.

## Notes

- The kernel still names no plugin: agents are found by what manifests declare, so
  `check-plugin-names.sh` keeps passing.
- No new storage. Agent settings stay core plugin settings (`settings.get` / `settings.set`),
  stored on the core; only where they are shown changes.
- No ADR: the protocol change is a new optional field (a minor change), and the rest is
  client layout.
