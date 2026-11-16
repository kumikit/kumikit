<!--
  Manage plugins (06-client, Plugin manager): core plugins on this core, and UI plugins
  and themes on this client. Part of the UI kernel, so a plugin that broke the UI can
  always be removed here.
-->
<script lang="ts">
  import type { InstalledPlugin } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { ClientPlugin } from '../plugins/client.svelte';
  import { describePermissions, startsPrograms } from '../plugins/permissions';
  import { trapFocus } from './dialog';

  let { kernel }: { kernel: Kernel } = $props();
  const core = $derived(kernel.corePlugins);
  const client = $derived(kernel.clientPlugins);

  /** The plugin whose permissions are being shown for a grant. */
  let granting = $state.raw<InstalledPlugin | undefined>();
  let removing = $state.raw<InstalledPlugin | undefined>();
  let trusting = $state.raw<ClientPlugin | undefined>();

  $effect(() => {
    // Refresh when the connection comes up and whenever the core's plugins change.
    if (kernel.connection.ready) {
      void kernel.connection.plugins;
      void core.refresh();
    }
  });

  async function installCore(kind: 'file' | 'folder') {
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
    if (!isComposing(e) && e.key === 'Escape') close();
  }

  const publisher = (id: string) => id.split('.')[0];
  const stateText: Record<string, string> = { installed: 'Not enabled', enabled: 'Enabled', disabled: 'Disabled' };
</script>

<div class="plugins">
  <h1>Plugins</h1>

  <section aria-labelledby="kk-core-plugins">
    <div class="head">
      <h2 id="kk-core-plugins">On this core <span class="kk-muted">({kernel.connection.hello?.core.name ?? 'local'})</span></h2>
      <button type="button" class="kk-button" disabled={!kernel.connection.ready} onclick={() => installCore('file')}>Install from file…</button>
      <button type="button" class="kk-button" disabled={!kernel.connection.ready} onclick={() => installCore('folder')}>Install from folder…</button>
    </div>
    {#if core.plugins.length === 0}
      <p class="kk-muted">{core.loading ? 'Loading…' : 'No core plugins are installed.'}</p>
    {/if}
    <ul class="list">
      {#each core.plugins as p (p.id)}
        <li class="card">
          <div class="title">
            <strong>{p.name}</strong>
            <span class="kk-muted">{p.id} · {p.version}</span>
            <span class="badge {p.state}">{stateText[p.state] ?? p.state}</span>
          </div>
          {#if p.description}<p>{p.description}</p>{/if}
          {#if p.failure}<p class="problem" role="alert">Failed: {p.failure.message}{p.failure.retryInSeconds ? ` (retrying in ${p.failure.retryInSeconds} s)` : ''}</p>{/if}
          {#if p.error}<p class="problem" role="alert">{p.error}</p>{/if}
          <ul class="perms" aria-label="Permissions">
            {#each describePermissions(p.granted ?? p.requested) as line (line)}<li>{line}</li>{/each}
          </ul>
          <div class="actions">
            {#if p.state !== 'enabled'}
              <button type="button" class="kk-button primary" onclick={() => (granting = p)}>Enable…</button>
            {:else}
              <button type="button" class="kk-button" onclick={() => core.disable(p.id)}>Disable</button>
            {/if}
            <button type="button" class="kk-button danger" onclick={() => (removing = p)}>Remove…</button>
          </div>
        </li>
      {/each}
    </ul>
  </section>

  <section aria-labelledby="kk-client-plugins">
    <div class="head">
      <h2 id="kk-client-plugins">On this client</h2>
      <button type="button" class="kk-button" onclick={installClient}>Install from file…</button>
    </div>
    <ul class="list">
      {#each client.plugins as p (p.entry.id)}
        <li class="card">
          <div class="title">
            <strong>{p.manifest.name}</strong>
            <span class="kk-muted">{p.entry.id} · {p.entry.version} · {p.kind === 'theme' ? 'theme' : 'UI plugin'}{p.entry.source === 'bundled' ? ' · bundled' : ''}</span>
            <span class="badge {p.enabled ? 'enabled' : 'disabled'}">{p.enabled ? 'Enabled' : p.needsTrust ? 'Not trusted' : 'Disabled'}</span>
          </div>
          {#if p.manifest.description}<p>{p.manifest.description}</p>{/if}
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
      <p class="kk-muted">A disabled UI plugin stops showing at once; its code is unloaded when the app restarts.</p>
    {/if}
  </section>
</div>

{#if granting}
  {@const p = granting}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-grant-title" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (granting = undefined))}>
    <h2 id="kk-grant-title">Enable {p.name}?</h2>
    <p class="kk-muted">{p.id} {p.version} from <strong>{publisher(p.id)}</strong>{#if p.repository} · {p.repository}{/if}</p>
    {#if !p.signed}<p class="warning">This package is not signed, so Kumikit cannot check who made it.</p>{/if}
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
      <button type="button" class="kk-button" onclick={() => { removing = undefined; void core.uninstall(p.id, false); }}>Remove, keep data</button>
      <button type="button" class="kk-button danger" onclick={() => { removing = undefined; void core.uninstall(p.id, true); }}>Remove and delete data</button>
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
      <button type="button" class="kk-button primary" onclick={() => { trusting = undefined; void client.trust(p.entry.id); }}>Trust and enable</button>
    </div>
  </div>
{/if}

<style>
  .plugins {
    height: 100%;
    overflow: auto;
    padding: 16px 24px;
  }
  h1 {
    margin: 0 0 12px;
    font-size: var(--kk-font-size-title, x-large);
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 16px 0 8px;
  }
  .head h2 {
    flex: 1;
  }
  .list {
    display: grid;
    gap: 8px;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .card {
    padding: 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px;
  }
  .card p {
    margin: 6px 0;
  }
  .badge {
    margin-left: auto;
    padding: 0 8px;
    border-radius: var(--kk-radius-small, 0);
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-raised, Canvas);
  }
  .badge.enabled {
    background: var(--kk-state-success-tint, Canvas);
  }
  .perms {
    margin: 6px 0;
    padding-left: 20px;
    color: var(--kk-text-secondary, CanvasText);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .problem,
  .warning {
    padding: 6px 8px;
    border-left: 3px solid var(--kk-state-warning, CanvasText);
    background: var(--kk-state-warning-tint, Canvas);
  }
  .problem {
    border-left-color: var(--kk-state-danger, CanvasText);
    background: var(--kk-state-danger-tint, Canvas);
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 12vh;
    left: 50%;
    display: grid;
    gap: 8px;
    width: min(520px, 92vw);
    transform: translateX(-50%);
    padding: 16px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  .dialog p {
    margin: 0;
  }
</style>
