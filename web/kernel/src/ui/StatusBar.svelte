<!-- Zed-inspired thin bottom status bar: global action icons (plugins, settings, theme,
     commands) on the left; plugin status items, pane count and core connection status on
     the right. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import Icon from './Icon.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  const conn = $derived(kernel.connection);
  const statusText: Record<string, string> = {
    ready: 'Connected',
    mismatch: 'Core version differs',
    reconnecting: 'Reconnecting…',
    connecting: 'Connecting…',
    unauthorized: 'Needs pairing',
    certificate_changed: 'Certificate changed',
  };
  const label = $derived(`${kernel.connections.length > 1 ? `${conn.label}: ` : ''}${statusText[conn.status] ?? conn.status}`);
  const modKey = $derived(kernel.mac ? '⌘' : 'Ctrl+');
</script>

<footer class="bar">
  <div class="group">
    <button
      type="button"
      class="bar-btn"
      class:active={kernel.main.kind === 'plugins'}
      aria-label="Manage plugins"
      title="Manage plugins and registry"
      onclick={() => (kernel.main = kernel.main.kind === 'plugins' ? { kind: 'tab' } : { kind: 'plugins' })}
    >
      <Icon name="plugins" size={12} />
    </button>
    <button
      type="button"
      class="bar-btn"
      class:active={kernel.main.kind === 'settings'}
      aria-label="Open settings"
      title={`Settings (${modKey},)`}
      onclick={() => (kernel.main = kernel.main.kind === 'settings' ? { kind: 'tab' } : { kind: 'settings' })}
    >
      <Icon name="settings" size={12} />
    </button>
    <button
      type="button"
      class="bar-btn"
      aria-label={kernel.theme.appearance === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
      title={kernel.theme.appearance === 'dark' ? 'Switch to light appearance' : 'Switch to dark appearance'}
      onclick={() => kernel.toggleAppearance()}
    >
      <Icon name={kernel.theme.appearance === 'dark' ? 'sun' : 'moon'} size={12} />
    </button>
    <button
      type="button"
      class="bar-btn"
      aria-label="Command palette"
      title={`Show all commands (${modKey}K)`}
      onclick={() => (kernel.dialog = { kind: 'palette' })}
    >
      <Icon name="command" size={12} />
    </button>
  </div>

  <span class="spacer"></span>

  <div class="group">
    {#each kernel.slotsIn('statusbar.item') as entry (entry)}
      <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={{}} slots={kernel.slots} />
    {/each}

    {#if kernel.paneCount > 1}
      <span class="meta" title="Active split panes">{kernel.paneCount} panes</span>
    {/if}

    <button
      type="button"
      class="conn-btn"
      class:down={conn.status !== 'ready'}
      title="Cores: add, remove, reconnect"
      onclick={() => (kernel.main = kernel.main.kind === 'cores' ? { kind: 'tab' } : { kind: 'cores' })}
    >
      <span class="kk-status {conn.status === 'ready' ? 'ready' : 'awaiting_approval'}" aria-hidden="true"></span>
      <span role="status" aria-live="polite">{label}</span>
    </button>
  </div>
</footer>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 6px;
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-frame, Canvas);
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    color: var(--kk-text-secondary, CanvasText);
    user-select: none;
    flex: none;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .bar-btn,
  .conn-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 18px;
    padding: 0 5px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  .bar-btn:hover,
  .conn-btn:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .bar-btn.active {
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, CanvasText);
  }
  .conn-btn.down {
    color: var(--kk-state-warning, CanvasText);
  }
  .meta {
    padding: 0 4px;
    color: var(--kk-text-muted, GrayText);
  }
  .spacer {
    flex: 1;
  }
</style>
