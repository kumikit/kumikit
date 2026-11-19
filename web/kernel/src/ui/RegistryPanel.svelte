<!--
  Plugins from registries, in the plugin manager (06-client, Plugin manager): find and
  install, the starter pack, updates, registries, lockfiles. Registries are contacted only
  by the buttons here (10-distribution, Fetching).
-->
<script lang="ts">
  import type { AvailableUpdate, InstalledPlugin, RegistryResult } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { ClientPlugin } from '../plugins/client.svelte';
  import { describePermissions, describeUiNeeds, shortKey } from '../plugins/permissions';
  import type { ImportItem } from '../plugins/registry.svelte';
  import type { ClientRegistryResult, ClientUpdate } from '../platform/index';
  import { trapFocus } from './dialog';

  let {
    kernel,
    starter = false,
    onCoreInstalled,
    onClientInstalled,
  }: {
    kernel: Kernel;
    starter?: boolean;
    onCoreInstalled: (p: InstalledPlugin) => void;
    onClientInstalled: (p: ClientPlugin) => void;
  } = $props();

  const reg = $derived(kernel.registries);
  const target = $derived(kernel.corePlugins.target);
  let query = $state('');
  /** An update that asks for more, waiting for the user's answer. */
  let widening = $state.raw<{ core?: AvailableUpdate; client?: ClientUpdate } | undefined>();
  /** A lockfile being shown for export, or a plan being shown for import. */
  let exported = $state.raw<{ title: string; text: string } | undefined>();
  let plan = $state.raw<{ side: 'core' | 'client'; items: ImportItem[] } | undefined>();
  /** Core plugins installed by an import, waiting for one grant. */
  let importedGrant = $state.raw<InstalledPlugin[] | undefined>();
  let newCoreRegistry = $state('');
  let newClientRegistry = $state('');
  let showRegistries = $state(false);

  const coreResults = $derived((reg.core ?? []).filter((r) => !starter || r.starter));
  const clientResults = $derived((reg.client ?? []).filter((r) => !starter || r.starter));
  const searched = $derived(reg.core !== undefined || reg.client !== undefined);

  async function installCore(r: RegistryResult) {
    const p = await reg.installCore(r);
    if (p) onCoreInstalled(p);
  }

  async function installClient(r: ClientRegistryResult) {
    const p = await reg.installClient(r);
    if (p) onClientInstalled(p);
  }

  async function updateCore(u: AvailableUpdate) {
    if (u.widens) widening = { core: u };
    else await reg.updateCore(u);
  }

  async function updateClient(u: ClientUpdate) {
    if (u.widens) widening = { client: u };
    else await reg.updateClient(u);
  }

  async function confirmWidening() {
    const w = widening;
    widening = undefined;
    if (w?.core) await reg.updateCore(w.core);
    if (w?.client) await reg.updateClient(w.client);
  }

  async function exportLock(side: 'core' | 'client') {
    const text = side === 'core' ? await reg.coreLock() : await reg.clientLock();
    if (text !== undefined) exported = { title: side === 'core' ? `Lockfile of ${target.label}` : 'Lockfile of this app', text };
  }

  async function importLock(side: 'core' | 'client') {
    const file = await kernel.platform.pickBytes('.lock,.toml,text/plain');
    if (!file) return;
    const text = new TextDecoder().decode(file.bytes);
    const items = await kernel.attempt('Cannot read the lockfile', () => (side === 'core' ? reg.corePlan(text) : reg.clientPlan(text)));
    if (items) plan = { side, items };
  }

  async function runImport() {
    const p = plan;
    plan = undefined;
    if (!p) return;
    if (p.side === 'core') {
      const toEnable = await reg.importCore(p.items);
      if (toEnable.length > 0) importedGrant = toEnable;
    } else {
      const n = await reg.importClient(p.items);
      kernel.notify(`Installed ${n} UI plugin(s) and theme(s)`, 'success');
    }
  }

  async function grantImported() {
    const ps = importedGrant ?? [];
    importedGrant = undefined;
    for (const p of ps) await kernel.corePlugins.enable(p);
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      kernel.notify('Copied', 'success');
    } catch {
      kernel.notify('Cannot copy here; select the text and copy it', 'warning');
    }
  }

  function onSearchKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Enter') void reg.search(query);
  }

  function onKey(e: KeyboardEvent, close: () => void) {
    if (!isComposing(e) && e.key === 'Escape') close();
  }

  const planText: Record<ImportItem['action'], string> = {
    install: 'will be installed',
    installed: 'already installed',
    'other-version': 'another version is installed; update it instead',
    'not-from-registry': 'installed from a file on the other machine; install it from that file',
    'unknown-registry': 'its registry is not configured here; add it first',
  };
  const kindText = (kind: string[]) => (kind.includes('theme') ? 'Theme' : kind.includes('ui') ? 'UI plugin' : 'Core plugin');
</script>

{#if reg.coreAvailable || reg.clientAvailable}
  <section aria-labelledby="kk-find-plugins">
    <div class="head">
      <h2 id="kk-find-plugins">{starter ? 'Starter pack' : 'Find plugins'}</h2>
      {#if !starter}
        <input class="kk-input" type="search" aria-label="Search the registries" placeholder="Search the registries" bind:value={query} onkeydown={onSearchKey} />
        <button type="button" class="kk-button" disabled={reg.searching} onclick={() => reg.search(query)}>Search</button>
      {:else}
        <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'plugins' })}>All plugins</button>
      {/if}
    </div>
    {#if starter}
      <p class="kk-muted">Common plugins from the registry. Each is installed only if you choose it, and shows what it may do first.</p>
    {/if}
    {#if reg.searching}
      <p class="kk-muted" role="status">Asking the registries…</p>
    {:else if searched && coreResults.length === 0 && clientResults.length === 0}
      <p class="kk-muted" role="status">Nothing found.</p>
    {/if}
    <ul class="list" aria-label="Registry results">
      {#each coreResults as r (r.registry + r.id)}
        <li class="card">
          <div class="title">
            <strong>{r.name}</strong>
            <span class="kk-muted">{r.id} · {r.version} · {kindText(r.kind)} · from {r.registryName}</span>
          </div>
          {#if r.description}<p>{r.description}</p>{/if}
          <p class="kk-muted">Published by <strong>{r.publisher}</strong>, signed with key <span title={r.key}>{shortKey(r.key)}</span>{#if r.repository} · {r.repository}{/if}</p>
          <ul class="perms" aria-label="Permissions">
            {#each describePermissions(r.requested) as line (line)}<li>{line}</li>{/each}
          </ul>
          <div class="actions">
            <button type="button" class="kk-button primary" disabled={kernel.corePlugins.plugins.some((p) => p.id === r.id)} onclick={() => installCore(r)}>
              {kernel.corePlugins.plugins.some((p) => p.id === r.id) ? 'Installed' : `Install on ${target.label}…`}
            </button>
          </div>
        </li>
      {/each}
      {#each clientResults as r (r.registry + r.id)}
        <li class="card">
          <div class="title">
            <strong>{r.name}</strong>
            <span class="kk-muted">{r.id} · {r.version} · {kindText(r.kind)} · from {r.registryName}</span>
          </div>
          {#if r.description}<p>{r.description}</p>{/if}
          <p class="kk-muted">Published by <strong>{r.publisher}</strong>, signed with key <span title={r.key}>{shortKey(r.key)}</span>{#if r.repository} · {r.repository}{/if}</p>
          <ul class="perms" aria-label="What it needs">
            {#each describeUiNeeds(r.kind, r.ui) as line (line)}<li>{line}</li>{/each}
          </ul>
          <div class="actions">
            <button type="button" class="kk-button primary" disabled={kernel.clientPlugins.plugins.some((p) => p.entry.id === r.id)} onclick={() => installClient(r)}>
              {kernel.clientPlugins.plugins.some((p) => p.entry.id === r.id) ? 'Installed' : 'Install in this app'}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  </section>

  {#if !starter}
    <section aria-labelledby="kk-updates">
      <div class="head">
        <h2 id="kk-updates">Updates</h2>
        <button type="button" class="kk-button" disabled={reg.checking} onclick={() => reg.checkUpdates()}>Check for updates</button>
      </div>
      {#if reg.coreUpdates !== undefined || reg.clientUpdates !== undefined}
        {#if (reg.coreUpdates ?? []).length === 0 && (reg.clientUpdates ?? []).length === 0}
          <p class="kk-muted" role="status">Everything is up to date.</p>
        {/if}
        <ul class="list" aria-label="Available updates">
          {#each reg.coreUpdates ?? [] as u (u.plugin)}
            <li class="card">
              <div class="title"><strong>{u.plugin}</strong><span class="kk-muted">{u.from} → {u.to} · on {target.label}</span></div>
              {#if u.widens}<p class="warning">This version asks for more permissions.</p>{/if}
              <div class="actions"><button type="button" class="kk-button" onclick={() => updateCore(u)}>{u.widens ? 'Review and update…' : 'Update'}</button></div>
            </li>
          {/each}
          {#each reg.clientUpdates ?? [] as u (u.plugin)}
            <li class="card">
              <div class="title"><strong>{u.plugin}</strong><span class="kk-muted">{u.from} → {u.to} · in this app</span></div>
              {#if u.widens}<p class="warning">This version needs more than the installed one.</p>{/if}
              <div class="actions"><button type="button" class="kk-button" onclick={() => updateClient(u)}>{u.widens ? 'Review and update…' : 'Update'}</button></div>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="kk-muted">Updates are never installed by themselves. Check when you want to.</p>
      {/if}
    </section>

    <section aria-labelledby="kk-registries">
      <div class="head">
        <h2 id="kk-registries">Registries and lockfiles</h2>
        <button type="button" class="kk-button" aria-expanded={showRegistries} onclick={() => { showRegistries = !showRegistries; if (showRegistries) void reg.loadRegistries(); }}>{showRegistries ? 'Hide' : 'Show'}</button>
      </div>
      {#if showRegistries}
        {#if reg.coreAvailable}
          <h3>On {target.label}</h3>
          <ul class="plain">
            {#each reg.coreRegistries ?? [] as r (r.url)}
              <li><span>{r.url}{r.name ? ` (${r.name})` : ''}</span> <button type="button" class="kk-button" onclick={() => reg.removeCoreRegistry(r.url)}>Remove</button></li>
            {/each}
          </ul>
          <p class="row">
            <input class="kk-input" aria-label="Registry URL for {target.label}" placeholder="https://…/index.json" bind:value={newCoreRegistry} />
            <button type="button" class="kk-button" disabled={!newCoreRegistry} onclick={() => { void reg.addCoreRegistry(newCoreRegistry); newCoreRegistry = ''; }}>Add</button>
            <button type="button" class="kk-button" onclick={() => exportLock('core')}>Export lockfile</button>
            <button type="button" class="kk-button" onclick={() => importLock('core')}>Import lockfile…</button>
          </p>
        {/if}
        {#if reg.clientAvailable}
          <h3>In this app</h3>
          <ul class="plain">
            {#each reg.clientRegistries ?? [] as url (url)}
              <li><span>{url}</span> <button type="button" class="kk-button" onclick={() => reg.setClientRegistries((reg.clientRegistries ?? []).filter((u) => u !== url))}>Remove</button></li>
            {/each}
          </ul>
          <p class="row">
            <input class="kk-input" aria-label="Registry URL for this app" placeholder="https://…/index.json" bind:value={newClientRegistry} />
            <button type="button" class="kk-button" disabled={!newClientRegistry} onclick={() => { void reg.setClientRegistries([...(reg.clientRegistries ?? []), newClientRegistry]); newClientRegistry = ''; }}>Add</button>
            <button type="button" class="kk-button" onclick={() => exportLock('client')}>Export lockfile</button>
            <button type="button" class="kk-button" onclick={() => importLock('client')}>Import lockfile…</button>
          </p>
        {/if}
      {/if}
    </section>
  {/if}
{/if}

{#if widening}
  {@const w = widening}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-widen-title" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (widening = undefined))}>
    {#if w.core}
      <h2 id="kk-widen-title">Update {w.core.plugin} to {w.core.to}?</h2>
      <p>This version asks for more permissions than the one you granted. Until you agree, {w.core.plugin} stays on {w.core.from}. The new version:</p>
      <ul class="perms">{#each describePermissions(w.core.requested) as line (line)}<li>{line}</li>{/each}</ul>
    {:else if w.client}
      <h2 id="kk-widen-title">Update {w.client.plugin} to {w.client.to}?</h2>
      <p>This version needs more than the installed one. The new version:</p>
      <ul class="perms">{#each describeUiNeeds(['ui'], w.client.ui) as line (line)}<li>{line}</li>{/each}</ul>
    {/if}
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (widening = undefined)}>Not now</button>
      <button type="button" class="kk-button primary" onclick={confirmWidening}>Grant and update</button>
    </div>
  </div>
{/if}

{#if exported}
  {@const e = exported}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-lock-title" tabindex="-1" use:trapFocus onkeydown={(ev) => onKey(ev, () => (exported = undefined))}>
    <h2 id="kk-lock-title">{e.title}</h2>
    <textarea class="kk-input lock" readonly aria-label="Lockfile">{e.text}</textarea>
    <div class="actions">
      <button type="button" class="kk-button" onclick={() => copy(e.text)}>Copy</button>
      <button type="button" class="kk-button primary" data-autofocus onclick={() => (exported = undefined)}>Close</button>
    </div>
  </div>
{/if}

{#if plan}
  {@const p = plan}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-import-title" tabindex="-1" use:trapFocus onkeydown={(ev) => onKey(ev, () => (plan = undefined))}>
    <h2 id="kk-import-title">Import a lockfile {p.side === 'core' ? `into ${target.label}` : 'into this app'}?</h2>
    <ul class="plain">
      {#each p.items as i (i.entry.id)}<li><strong>{i.entry.id}</strong> {i.entry.version}: {planText[i.action]}</li>{/each}
    </ul>
    <p class="kk-muted">Each package must have the hash the lockfile records. You are asked before anything is granted.</p>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (plan = undefined)}>Cancel</button>
      <button type="button" class="kk-button primary" disabled={!p.items.some((i) => i.action === 'install')} onclick={runImport}>Install</button>
    </div>
  </div>
{/if}

{#if importedGrant}
  {@const ps = importedGrant}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-import-grant-title" tabindex="-1" use:trapFocus onkeydown={(ev) => onKey(ev, () => (importedGrant = undefined))}>
    <h2 id="kk-import-grant-title">Enable the imported plugins on {target.label}?</h2>
    <p>They were enabled on the other machine. Together they ask for:</p>
    {#each ps as p (p.id)}
      <p><strong>{p.name}</strong> ({p.id})</p>
      <ul class="perms">{#each describePermissions(p.requested) as line (line)}<li>{line}</li>{/each}</ul>
    {/each}
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (importedGrant = undefined)}>Not now</button>
      <button type="button" class="kk-button primary" onclick={grantImported}>Grant all and enable</button>
    </div>
  </div>
{/if}

<style>
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  h3 {
    margin: 12px 0 4px;
    font-size: var(--kk-font-size-base, medium);
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
  .plain {
    margin: 4px 0;
    padding-left: 20px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
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
  .warning {
    padding: 6px 8px;
    border-left: 3px solid var(--kk-state-warning, CanvasText);
    background: var(--kk-state-warning-tint, Canvas);
  }
  .lock {
    min-height: 240px;
    font-family: var(--kk-font-mono, monospace);
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 10vh;
    left: 50%;
    display: grid;
    gap: 8px;
    width: min(560px, 92vw);
    max-height: 80vh;
    overflow: auto;
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
