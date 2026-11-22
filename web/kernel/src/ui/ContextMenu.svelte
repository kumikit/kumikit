<!-- CSP-safe floating context menu positioned via a constructed stylesheet (no inline
     `style` attributes). Closes on outside pointerdown, Escape, or window blur/resize. -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import Icon, { type IconName } from './Icon.svelte';

  export interface ContextMenuItem {
    id: string;
    label: string;
    icon?: IconName;
    shortcut?: string;
    tone?: 'default' | 'danger';
    disabled?: boolean;
    onSelect: () => void;
  }

  let {
    x,
    y,
    label,
    items,
    onClose,
  }: {
    x: number;
    y: number;
    label: string;
    items: ContextMenuItem[];
    onClose: () => void;
  } = $props();

  let menuEl: HTMLElement | undefined = $state();
  const menuId = `kk-ctx-${Math.random().toString(36).slice(2, 8)}`;

  // Position through a constructed stylesheet so CSP `style-src 'self'` stays strict.
  $effect(() => {
    const rawX = x;
    const rawY = y;
    if (typeof document === 'undefined' || !('adoptedStyleSheets' in document)) return;
    const w = menuEl?.offsetWidth || 200;
    const h = menuEl?.offsetHeight || items.length * 32 + 12;
    const maxX = Math.max(8, window.innerWidth - w - 8);
    const maxY = Math.max(8, window.innerHeight - h - 8);
    const left = Math.round(Math.max(8, Math.min(rawX, maxX)));
    const top = Math.round(Math.max(8, Math.min(rawY, maxY)));
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`[data-ctx-menu="${menuId}"] { left: ${left}px; top: ${top}px; }`);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    return () => {
      document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet);
    };
  });

  function onPointerDown(e: PointerEvent) {
    if (menuEl && !menuEl.contains(e.target as Node)) onClose();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  }
</script>

<svelte:window onpointerdown={onPointerDown} onkeydown={onKeyDown} onblur={onClose} onresize={onClose} />

<div
  class="kk-ctx-menu"
  data-ctx-menu={menuId}
  role="menu"
  aria-label={label}
  bind:this={menuEl}
>
  {#each items as item (item.id)}
    <button
      type="button"
      class="kk-ctx-item"
      class:danger={item.tone === 'danger'}
      role="menuitem"
      disabled={item.disabled}
      onclick={() => {
        onClose();
        item.onSelect();
      }}
    >
      {#if item.icon}
        <span class="ico"><Icon name={item.icon} size={13} /></span>
      {/if}
      <span class="lbl">{item.label}</span>
      {#if item.shortcut}
        <kbd class="kbd">{item.shortcut}</kbd>
      {/if}
    </button>
  {/each}
</div>

<style>
  .kk-ctx-menu {
    position: fixed;
    z-index: 60;
    min-width: 184px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-popover, none);
  }
  .kk-ctx-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
  }
  .kk-ctx-item:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .kk-ctx-item.danger {
    color: var(--kk-state-danger, CanvasText);
  }
  .kk-ctx-item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: inherit;
    flex: none;
  }
  .lbl {
    flex: 1;
    white-space: nowrap;
  }
  .kbd {
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
</style>
