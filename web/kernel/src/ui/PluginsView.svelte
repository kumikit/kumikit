<!--
  Manage plugins inside the overlay sheet (06-client, Plugin manager): registry discovery,
  core plugins on a chosen core, and UI plugins & themes on this client. Part of the UI
  kernel, so a plugin that broke the UI can always be removed here.
-->
<script lang="ts">
  import type { InstalledPlugin } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { ClientPlugin } from '../plugins/client.svelte';
  import { describePermissions, startsPrograms } from '../plugins/permissions';
  import { trapFocus } from './dialog';
  import LinkedWorkspaces from './LinkedWorkspaces.svelte';
  import RegistryPanel from './RegistryPanel.svelte';

  let { kernel, starter = false }: { kernel: Kernel; starter?: boolean } = $props();
  const core = $derived(kernel.corePlugins);
  const client = $derived(kernel.clientPlugins);

  /** The plugin whose permissions are being shown for a grant. */
  let granting = $state.raw<InstalledPlugin | undefined>();
  let removing = $state.raw<InstalledPlugin | undefined>();
  let trusting = $state.raw<ClientPlugin | undefined>();
  let scroller: HTMLElement | undefined = $state();

  const target = $derived(core.target);

  $effect(() => {
    // Refresh when the chosen core comes up and whenever its plugins change.
    if (target.ready) {
      void target.plugins;
      void core.refresh();
    }
  });

  function jumpTo(id: string) {
    scroller?.querySelector(`#${id}`)?.scrollIntoView({ behavior: 'auto', block: 'start' });
  }

  async function installCore(kind: 'file' | 'folder') {
    if (core.uploads) {
      const file = await kernel.platform.pickBytes('.kkp');
      if (!file) return;
      const p = await core.upload(file.bytes);
      if (p) granting = p;
      return;
    }
    const path =
      kind === 'file'
        ? await kernel.platform.pickFile('Install a core plugin', ['kkp'])
        : await kernel.platform.pickFolder('Install a plugin folder (with a plugin.toml)');
    if (!path) return;
    const p = await core.install(path);
    if (p) granting = p;
  }

  async function grant() {
    const p = granting;
    granting = undefined;
    if (p) await core.enable(p);
  }

  async function installClient() {
    const path = await kernel.platform.pickFile('Install a UI plugin or theme', ['kkp']);
    if (!path) return;
    const p = await client.install(path);
    if (p?.needsTrust) trusting = p;
  }

  function onKey(e: KeyboardEvent, close: () => void) {
    if (!isComposing(e) && e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }

  const publisher = (id: string) => id.split('.')[0];
  const stateText: Record<string, string> = { installed: 'Not enabled', enabled: 'Enabled', disabled: 'Disabled' };
</script>

<div class="plugins-shell">
  <div class="subbar">
    <div class="pills" role="group" aria-label="Jump to section">
      {#if kernel.registries.coreAvailable || kernel.registries.clientAvailable}
        <button type="button" class="pill" onclick={() => jumpTo('kk-find-plugins')}>{starter ? 'Starter pack' : 'Discover'}</button>
      {/if}
      <button type="button" class="pill" onclick={() => jumpTo('kk-core-plugins')}>Core ({core.plugins.length})</button>
      <button type="button" class="pill" onclick={() => jumpTo('kk-client-plugins')}>Client ({client.plugins.length})</button>
      {#if !starter && (kernel.registries.coreAvailable || kernel.registries.clientAvailable)}
        <button type="button" class="pill" onclick={() => jumpTo('kk-updates')}>Updates</button>
        <button type="button" class="pill" onclick={() => jumpTo('kk-registries')}>Registries</button>
      {/if}
    </div>
    {#if kernel.connections.length > 1}
      <label class="choose">
        <span class="kk-muted">Core</span>
        <select class="kk-input compact-select" value={target.id} onchange={(e) => core.choose(e.currentTarget.value)}>
          {#each kernel.connections as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
        </select>
      </label>
    {/if}
  </div>

  <div class="plugins" bind:this={scroller}>
    <RegistryPanel
      {kernel}
      {starter}
      onCoreInstalled={(p) => (granting = p)}
      onClientInstalled={(p) => {
        if (p.needsTrust) trusting = p;
      }}
    />

    <section class="group" aria-labelledby="kk-core-plugins">
      <div class="head">
        <div class="head-title">
          <h2 id="kk-core-plugins">On this core</h2>
          {#if kernel.connections.length > 1}
            <label class="choose inline-choose">Core
              <select class="kk-input compact-select" value={target.id} onchange={(e) => core.choose(e.currentTarget.value)}>
                {#each kernel.connections as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
              </select>
            </label>
          {:else}
            <span class="meta-chip">{target.hello?.core.name ?? target.label}</span>
          {/if}
        </div>
        <div class="head-actions">
          <button type="button" class="kk-button" disabled={!target.ready} onclick={() => installCore('file')}>Install from file…</button>
          {#if !core.uploads}
            <button type="button" class="kk-button" disabled={!target.ready} onclick={() => installCore('folder')}>Install from folder…</button>
          {/if}
        </div>
      </div>
      {#if core.uploads}
        <p class="kk-muted note">{target.label} is on another machine: a package file is sent to it. To install from a folder there, pack it first with <code>kumikit plugin pack</code>.</p>
      {/if}
      {#if core.plugins.length === 0}
        <p class="kk-muted note">{core.loading ? 'Loading…' : 'No core plugins are installed.'}</p>
      {/if}
      <ul class="list">
        {#each core.plugins as p (p.id)}
          <li class="card">
            <div class="card-top">
              <div class="title-col">
                <strong>{p.name}</strong>
                <span class="sub-id">{p.id} · v{p.version}{p.source === 'bundled' ? ' · bundled' : ''}</span>
              </div>
              <span class="badge {p.state}">
                <span class="badge-dot {p.state}" aria-hidden="true"></span>
                {stateText[p.state] ?? p.state}
              </span>
            </div>
            {#if p.description}<p class="desc">{p.description}</p>{/if}
            {#if p.failure}<p class="problem" role="alert">Failed: {p.failure.message}{p.failure.retryInSeconds ? ` (retrying in ${p.failure.retryInSeconds} s)` : ''}</p>{/if}
            {#if p.error}<p class="problem" role="alert">{p.error}</p>{/if}
            <ul class="perms" aria-label="Permissions">
              {#each describePermissions(p.granted ?? p.requested) as line (line)}<li>{line}</li>{/each}
            </ul>
            {#if p.state === 'enabled' && p.granted?.workspaces === 'linked'}
              <LinkedWorkspaces connection={core.target} plugin={p.id} notify={(m) => kernel.notify(m, 'danger')} />
            {/if}
            <div class="actions">
              {#if p.state !== 'enabled'}
                <button type="button" class="kk-button primary" onclick={() => (granting = p)}>Enable…</button>
              {:else}
                <button type="button" class="kk-button" onclick={() => core.disable(p.id)}>Disable</button>
              {/if}
              {#if p.source !== 'bundled'}
                <button type="button" class="kk-button danger" onclick={() => (removing = p)}>Remove…</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>

    <section class="group" aria-labelledby="kk-client-plugins">
      <div class="head">
        <div class="head-title">
          <h2 id="kk-client-plugins">On this client</h2>
          <span class="meta-chip">{client.plugins.length} installed</span>
        </div>
        <div class="head-actions">
          {#if kernel.platform.kind !== 'browser'}
            <button type="button" class="kk-button" onclick={installClient}>Install from file…</button>
          {/if}
        </div>
      </div>
      <ul class="list">
        {#each client.plugins as p (p.entry.id)}
          <li class="card">
            <div class="card-top">
              <div class="title-col">
                <strong>{p.manifest.name}</strong>
                <span class="sub-id">{p.entry.id} · v{p.entry.version} · {p.kind === 'theme' ? 'theme' : 'UI plugin'}{p.entry.source === 'bundled' ? ' · bundled' : ''}</span>
              </div>
              <span class="badge {p.enabled ? 'enabled' : 'disabled'}">
                <span class="badge-dot {p.enabled ? 'enabled' : 'disabled'}" aria-hidden="true"></span>
                {p.enabled ? 'Enabled' : p.needsTrust ? 'Not trusted' : 'Disabled'}
              </span>
            </div>
            {#if p.manifest.description}<p class="desc">{p.manifest.description}</p>{/if}
            {#if p.entry.publisher}<p class="sig">Signed with key <span title={p.entry.publisher.key}>{p.entry.publisher.key.slice(0, 20)}…</span>{p.entry.publisher.registry ? ` · from ${p.entry.publisher.registry}` : ''}</p>{/if}
            {#if p.error}<p class="problem" role="alert">{p.error}</p>{/if}
            <div class="actions">
              {#if p.needsTrust}
                <button type="button" class="kk-button primary" onclick={() => (trusting = p)}>Trust and enable…</button>
              {:else}
                <button type="button" class="kk-button" onclick={() => client.setEnabled(p.entry.id, !p.enabled)}>{p.enabled ? 'Disable' : 'Enable'}</button>
              {/if}
              {#if p.entry.source === 'installed'}
                <button type="button" class="kk-button danger" onclick={() => client.remove(p.entry.id)}>Remove</button>
              {/if}
            </div>
          </li>
        {:else}
          <li class="kk-muted">No UI plugins or themes.</li>
        {/each}
      </ul>
      {#if client.plugins.some((p) => p.kind === 'ui')}
        <p class="kk-muted note">A disabled UI plugin stops showing at once; its code is unloaded when the app restarts.</p>
      {/if}
    </section>
  </div>
</div>

{#if granting}
  {@const p = granting}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-grant-title" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (granting = undefined))}>
    <h2 id="kk-grant-title">Enable {p.name} on {target.label}?</h2>
    <p class="kk-muted">{p.id} {p.version} from <strong>{publisher(p.id)}</strong>{#if p.repository} · {p.repository}{/if}</p>
    {#if p.signed && p.publisher}
      <p>Signed with key <code class="key">{p.publisher.key}</code>{#if p.publisher.registry}, the publisher’s key in the registry {p.publisher.registry}.{:else}; not checked against a registry.{/if}</p>
    {:else}
      <p class="warning">This package is not signed, so Kumikit cannot check who made it.</p>
    {/if}
    {#if startsPrograms(p.requested)}
      <p class="warning">It starts programs outside the sandbox. They run with your full user access, so they can read and change anything you can.</p>
    {/if}
    <p>It asks for these permissions, granted together:</p>
    <ul class="perms">
      {#each describePermissions(p.requested) as line (line)}<li>{line}</li>{/each}
    </ul>
    <div class="actions">
      <button type="button" class="kk-button" onclick={() => (granting = undefined)}>Not now</button>
      <button type="button" class="kk-button primary" data-autofocus onclick={grant}>Grant and enable</button>
    </div>
  </div>
{/if}

{#if removing}
  {@const p = removing}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-remove-title" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (removing = undefined))}>
    <h2 id="kk-remove-title">Remove {p.name}?</h2>
    <p>Its running turns are cancelled. Keep its data (settings and stored state) for a later install, or delete it too.</p>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (removing = undefined)}>Cancel</button>
      <button type="button" class="kk-button" onclick={() => { const id = p.id; removing = undefined; void core.uninstall(id, false); }}>Remove, keep data</button>
      <button type="button" class="kk-button danger" onclick={() => { const id = p.id; removing = undefined; void core.uninstall(id, true); }}>Remove and delete data</button>
    </div>
  </div>
{/if}

{#if trusting}
  {@const p = trusting}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-trust-title" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (trusting = undefined))}>
    <h2 id="kk-trust-title">Trust {p.manifest.name}?</h2>
    <p class="warning">This UI plugin runs its code inside the app, with full access to this app and to every core it is connected to. Trust it only if you trust its publisher, <strong>{publisher(p.entry.id)}</strong>.</p>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (trusting = undefined)}>Not now</button>
      <button type="button" class="kk-button primary" onclick={() => { const id = p.entry.id; trusting = undefined; void client.trust(id); }}>Trust and enable</button>
    </div>
  </div>
{/if}

<style>
  .plugins-shell {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .subbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 20px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-panel, Canvas);
    flex: none;
  }
  .pills {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pill {
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, ButtonFace);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .pill:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .plugins {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 18px 24px 28px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    color: var(--kk-text-primary, CanvasText);
  }
  .choose {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--kk-font-size-small, small);
  }
  .compact-select {
    width: auto;
    padding: 3px 8px;
    font-size: var(--kk-font-size-small, small);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
  }
  .head-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .meta-chip {
    padding: 1px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
  }
  .note {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
    gap: 10px;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .card-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
  }
  .title-col {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .sub-id {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow-wrap: anywhere;
  }
  .desc {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .sig {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 1px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    white-space: nowrap;
    flex: none;
  }
  .badge.enabled {
    background: var(--kk-state-success-tint, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .badge-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-text-muted, GrayText);
  }
  .badge-dot.enabled {
    background: var(--kk-state-success, Highlight);
  }
  .perms {
    margin: 0;
    padding-left: 18px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: auto;
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .problem,
  .warning {
    margin: 0;
    padding: 6px 10px;
    border-left: 3px solid var(--kk-state-warning, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-state-warning-tint, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .problem {
    border-left-color: var(--kk-state-danger, CanvasText);
    background: var(--kk-state-danger-tint, Canvas);
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 12vh;
    left: 50%;
    z-index: 41;
    display: grid;
    gap: 10px;
    width: min(520px, 92vw);
    transform: translateX(-50%);
    padding: 18px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  .dialog p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .dialog .actions {
    border-top: 0;
    padding-top: 4px;
  }
  .key {
    overflow-wrap: anywhere;
  }
</style>
