<!--
  Settings → Agents (06-client, Agents): the agent providers on a chosen core, installed and
  offered by its registries, with each agent's settings in place. Part of the UI kernel, so
  it works in safe mode. Registries are contacted only by the buttons here
  (10-distribution, Fetching).
-->
<script lang="ts">
  import type { AvailableUpdate, InstalledPlugin, RegistryResult } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import { isAgent, updatesFor } from '../plugins/agents';
  import { describePermissions, shortKey } from '../plugins/permissions';
  import CorePluginDialog, { type CorePluginAsk } from './CorePluginDialog.svelte';
  import CoreSettings from './CoreSettings.svelte';
  import LinkedWorkspaces from './LinkedWorkspaces.svelte';
  import ProviderIcon from './ProviderIcon.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const core = $derived(kernel.corePlugins);
  const reg = $derived(kernel.registries);
  const target = $derived(core.target);
  const agents = $derived(core.plugins.filter(isAgent));
  const updates = $derived(updatesFor(reg.coreUpdates ?? [], core.plugins, true));
  const available = $derived((reg.agents ?? []).filter((r) => !core.plugins.some((p) => p.id === r.id)));
  const hasSettings = $derived(target.hello?.capabilities.includes('settings') ?? false);

  let asking = $state.raw<CorePluginAsk | undefined>();
  let query = $state('');

  $effect(() => {
    // Refresh when the chosen core comes up and whenever its plugins change.
    if (target.ready) {
      void target.plugins;
      void core.refresh();
    }
  });

  const grantFor = (plugin: InstalledPlugin) => (asking = { kind: 'grant', plugin });
  const updateOf = (id: string) => updates.find((u) => u.plugin === id);
  const stateText: Record<string, string> = { installed: 'Not enabled', enabled: 'Enabled', disabled: 'Disabled' };

  /** The providers to show for an agent: what its manifest declares. */
  function providersOf(p: InstalledPlugin): { id: string; name: string; icon?: string }[] {
    if (p.providers?.length) return p.providers;
    return p.contributes.providers.map((id) => ({ id, name: p.name }));
  }

  async function installFile(kind: 'file' | 'folder') {
    if (core.uploads) {
      const file = await kernel.platform.pickBytes('.kkp');
      if (!file) return;
      const p = await core.upload(file.bytes);
      if (p) grantFor(p);
      return;
    }
    const path =
      kind === 'file'
        ? await kernel.platform.pickFile('Install an agent', ['kkp'])
        : await kernel.platform.pickFolder('Install an agent folder (with a plugin.toml)');
    if (!path) return;
    const p = await core.install(path);
    if (p) grantFor(p);
  }

  async function install(r: RegistryResult) {
    const p = await reg.installCore(r);
    if (p) grantFor(p);
  }

  async function update(u: AvailableUpdate) {
    if (u.widens) asking = { kind: 'widen', update: u };
    else await reg.updateCore(u);
  }

  function onSearchKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Enter') void reg.searchAgents(query);
  }
</script>

<div class="agents">
  <div class="head">
    <div>
      <h2>Agents</h2>
      <p class="kk-muted sub">The coding agents on {target.label}, and their settings.</p>
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

  <section aria-labelledby="kk-agents-installed">
    <div class="section-head">
      <h3 id="kk-agents-installed">Installed</h3>
      <div class="head-actions">
        {#if reg.coreAvailable}
          <button type="button" class="kk-button" disabled={reg.checking || !target.ready} onclick={() => reg.checkUpdates()}>Check for updates</button>
        {/if}
        <button type="button" class="kk-button" disabled={!target.ready} onclick={() => installFile('file')}>Install from file…</button>
        {#if !core.uploads}
          <button type="button" class="kk-button" disabled={!target.ready} onclick={() => installFile('folder')}>Install from folder…</button>
        {/if}
      </div>
    </div>
    {#if agents.length === 0}
      <p class="kk-muted note">{core.loading ? 'Loading…' : `No agent is installed on ${target.label}. Find one below, or install one from a file.`}</p>
    {/if}
    <ul class="list" aria-label="Installed agents">
      {#each agents as p (p.id)}
        {@const u = updateOf(p.id)}
        <li class="card">
          <div class="card-top">
            <div class="title">
              <span class="icons">
                {#each providersOf(p) as v (v.id)}<ProviderIcon name={v.name} icon={v.icon} size="medium" />{/each}
              </span>
              <div class="title-col">
                <strong>{p.name}</strong>
                <span class="sub-id">{p.id} · v{p.version}{p.source === 'bundled' ? ' · bundled' : ''}</span>
              </div>
            </div>
            <span class="badge {p.state}">
              <span class="badge-dot {p.state}" aria-hidden="true"></span>
              {stateText[p.state] ?? p.state}
            </span>
          </div>
          {#if p.description}<p class="desc">{p.description}</p>{/if}
          {#if providersOf(p).length > 1 || providersOf(p)[0]?.name !== p.name}
            <p class="desc">Provides {providersOf(p).map((v) => v.name).join(', ')}</p>
          {/if}
          {#if p.failure}<p class="problem" role="alert">Failed: {p.failure.message}{p.failure.retryInSeconds ? ` (retrying in ${p.failure.retryInSeconds} s)` : ''}</p>{/if}
          {#if p.error}<p class="problem" role="alert">{p.error}</p>{/if}
          {#if u}
            <p class="update" role="status">Version {u.to} is available{u.widens ? '; it asks for more permissions' : ''}.</p>
          {/if}
          <details class="more">
            <summary>Permissions</summary>
            <ul class="perms" aria-label="Permissions">
              {#each describePermissions(p.granted ?? p.requested) as line (line)}<li>{line}</li>{/each}
            </ul>
          </details>
          {#if p.state === 'enabled' && hasSettings && p.contributes.settings}
            <details class="more settings">
              <summary>Settings</summary>
              {#key p.id}
                <CoreSettings connection={target} plugin={p.id} notify={(m) => kernel.notify(m, 'danger')} />
              {/key}
            </details>
          {/if}
          {#if p.state === 'enabled' && p.granted?.workspaces === 'linked'}
            <LinkedWorkspaces connection={target} plugin={p.id} notify={(m) => kernel.notify(m, 'danger')} />
          {/if}
          <div class="actions">
            {#if u}
              <button type="button" class="kk-button" onclick={() => update(u)}>{u.widens ? 'Review and update…' : 'Update'}</button>
            {/if}
            {#if p.state !== 'enabled'}
              <button type="button" class="kk-button primary" onclick={() => grantFor(p)}>Enable…</button>
            {:else}
              <button type="button" class="kk-button" onclick={() => core.disable(p.id)}>Disable</button>
            {/if}
            {#if p.source !== 'bundled'}
              <button type="button" class="kk-button danger" onclick={() => (asking = { kind: 'remove', plugin: p })}>Remove…</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  </section>

  {#if reg.coreAvailable}
    <section aria-labelledby="kk-agents-available">
      <div class="section-head">
        <h3 id="kk-agents-available">Available</h3>
      </div>
      <div class="search-row">
        <input
          class="kk-input search-input"
          type="search"
          placeholder="Search agents in the registries"
          aria-label="Search agents"
          bind:value={query}
          onkeydown={onSearchKey}
        />
        <button type="button" class="kk-button primary" disabled={reg.searchingAgents || !target.ready} onclick={() => reg.searchAgents(query)}>
          {reg.agents === undefined && query.trim() === '' ? 'Browse agents' : 'Search'}
        </button>
      </div>
      {#if reg.searchingAgents}
        <p class="kk-muted note" role="status">Asking the registries…</p>
      {:else if reg.agents === undefined}
        <p class="kk-muted note">The registries of {target.label} are asked only when you browse or search.</p>
      {:else if available.length === 0}
        <p class="kk-muted note" role="status">{reg.agents.length > 0 ? 'Every agent found is installed already.' : 'No agents found.'}</p>
      {/if}
      {#if available.length > 0}
        <ul class="list" aria-label="Available agents">
          {#each available as r (r.registry + r.id)}
            <li class="card">
              <div class="card-top">
                <div class="title">
                  <span class="icons"><ProviderIcon name={r.name} size="medium" /></span>
                  <div class="title-col">
                    <strong>{r.name}</strong>
                    <span class="sub-id">{r.id} · v{r.version} · {r.registryName}</span>
                  </div>
                </div>
              </div>
              {#if r.description}<p class="desc">{r.description}</p>{/if}
              <p class="sig">Published by <strong>{r.publisher}</strong>, signed with key <span title={r.key}>{shortKey(r.key)}</span>{#if r.repository} · {r.repository}{/if}</p>
              <details class="more">
                <summary>Permissions</summary>
                <ul class="perms" aria-label="Permissions">
                  {#each describePermissions(r.requested) as line (line)}<li>{line}</li>{/each}
                </ul>
              </details>
              <div class="actions">
                <button type="button" class="kk-button primary" onclick={() => install(r)}>Install…</button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

{#if asking}
  <CorePluginDialog
    ask={asking}
    core={target.label}
    onclose={() => (asking = undefined)}
    ongrant={(p) => void core.enable(p)}
    onremove={(p, deleteData) => void core.uninstall(p.id, deleteData)}
    onupdate={(u) => void reg.updateCore(u)}
  />
{/if}

<style>
  .agents {
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: var(--kk-font-size-large, large);
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-base, medium);
  }
  .sub,
  .note {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
  }
  .head-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
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
  .search-row {
    display: flex;
    gap: 6px;
  }
  .search-input {
    flex: 1;
  }
  .list {
    display: flex;
    flex-direction: column;
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
  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .icons {
    display: inline-flex;
    gap: 4px;
    flex: none;
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
  .desc,
  .sig {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .sig {
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
  .more summary {
    cursor: pointer;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .more[open] summary {
    margin-bottom: 8px;
  }
  .settings[open] {
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-panel, Canvas);
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
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .problem,
  .update {
    margin: 0;
    padding: 6px 10px;
    border-left: 3px solid var(--kk-state-danger, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-state-danger-tint, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .update {
    border-left-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-raised, Canvas);
  }
</style>
