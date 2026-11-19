<!-- The window: sidebar, tabs, the main area and the status bar. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import NewThread from './NewThread.svelte';
  import Palette from './Palette.svelte';
  import PluginsView from './PluginsView.svelte';
  import CoresView from './CoresView.svelte';
  import OpenFolder from './OpenFolder.svelte';
  import PairBrowser from './PairBrowser.svelte';
  import SettingsView from './SettingsView.svelte';
  import Sidebar from './Sidebar.svelte';
  import SlotHost from './SlotHost.svelte';
  import StatusBar from './StatusBar.svelte';
  import TabBar from './TabBar.svelte';
  import ThreadPane from './ThreadPane.svelte';
  import Toasts from './Toasts.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  // In a browser served by a core, an unpaired browser sees only the pairing screen.
  const pairing = $derived(kernel.platform.browser !== undefined && kernel.connections[0]?.status === 'unauthorized');
  const mainView = $derived(kernel.main);
  const pluginView = $derived(
    mainView.kind === 'view' ? kernel.slots.in('view').find((v) => v.contribution.id === mainView.id) : undefined,
  );

  function onKey(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229 || kernel.dialog) return;
    const cmd = kernel.commands.forKey(e, kernel.when, kernel.mac, kernel.commandAllowed);
    if (!cmd) return;
    e.preventDefault();
    void kernel.attempt('The command failed', () => Promise.resolve(cmd.run()));
  }
</script>

<svelte:window onkeydown={onKey} />

{#if pairing}
  <PairBrowser {kernel} />
{:else}
<div class="frame">
  <aside><Sidebar {kernel} /></aside>
  <main>
    {#if kernel.platform.safeMode}
      <div class="banner" role="status">Safe mode: no UI plugins are loaded. Remove a broken plugin in Manage plugins, then restart.</div>
    {/if}
    <TabBar {kernel} />
    <div class="content">
      {#if mainView.kind === 'settings'}
        <SettingsView {kernel} page={mainView.page} />
      {:else if mainView.kind === 'plugins'}
        <PluginsView {kernel} starter={mainView.starter ?? false} />
      {:else if mainView.kind === 'cores'}
        <CoresView {kernel} />
      {:else if mainView.kind === 'view' && pluginView}
        <SlotHost plugin={pluginView.plugin} contribution={pluginView.contribution} props={{ args: mainView.args }} slots={kernel.slots} label={pluginView.contribution.title} />
      {:else}
        <div class="pane"><ThreadPane {kernel} /></div>
      {/if}
    </div>
  </main>
  <StatusBar {kernel} />
</div>

{/if}

{#if kernel.dialog?.kind === 'palette'}
  <Palette {kernel} />
{:else if kernel.dialog?.kind === 'new-thread'}
  <NewThread {kernel} />
{:else if kernel.dialog?.kind === 'open-folder'}
  <OpenFolder {kernel} connection={kernel.dialog.connection} />
{/if}
<Toasts {kernel} />

<style>
  .frame {
    display: grid;
    grid-template-columns: minmax(200px, 260px) 1fr;
    grid-template-rows: 1fr auto;
    height: 100%;
  }
  aside {
    grid-row: 1;
    min-height: 0;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--kk-surface-panel, Canvas);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .frame > :global(footer) {
    grid-column: 1 / -1;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    background: var(--kk-state-warning-tint, Mark);
    color: var(--kk-text-primary, MarkText);
  }
</style>
