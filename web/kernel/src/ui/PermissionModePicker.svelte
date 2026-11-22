<!-- Permission mode picker: compact padlock/shield icon button anchored in the composer
     footer that opens an upward popover menu with Standard, Accept edits, Plan mode, and
     Full access. Theme-tokenized for both light and dark appearances. -->
<script lang="ts">
  import type { PermissionMode } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';

  let {
    value = 'standard',
    disabled = false,
    onChange,
  }: {
    value: PermissionMode;
    disabled?: boolean;
    onChange: (mode: PermissionMode) => void;
  } = $props();

  let open = $state(false);
  let root: HTMLElement | undefined = $state();

  interface ModeSpec {
    id: PermissionMode;
    label: string;
    description: string;
  }

  const MODES: ModeSpec[] = [
    {
      id: 'standard',
      label: 'Standard',
      description: 'Ask before edits and commands',
    },
    {
      id: 'accept_edits',
      label: 'Accept edits',
      description: 'Auto-approve workspace edits; ask before running commands',
    },
    {
      id: 'plan',
      label: 'Plan mode',
      description: 'Read-only planning and exploration before making changes',
    },
    {
      id: 'full_access',
      label: 'Full access',
      description: 'Allow edits and commands without asking',
    },
  ];

  const current = $derived(MODES.find((m) => m.id === value) ?? MODES[0]!);

  function choose(mode: PermissionMode) {
    open = false;
    if (mode !== value) onChange(mode);
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (!open) return;
    if (root && !root.contains(e.target as Node)) open = false;
  }

  function onWindowKeyDown(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (open && e.key === 'Escape') {
      e.preventDefault();
      open = false;
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeyDown} />

<div class="perm-wrap" bind:this={root}>
  <button
    type="button"
    class="perm-trigger"
    class:active={open}
    class:tone-standard={value === 'standard'}
    class:tone-accept={value === 'accept_edits'}
    class:tone-plan={value === 'plan'}
    class:tone-full={value === 'full_access'}
    aria-label={`Permissions: ${current.label}`}
    aria-haspopup="menu"
    aria-expanded={open}
    title={`Permissions: ${current.label} — ${current.description}`}
    {disabled}
    onclick={() => (open = !open)}
  >
    {#if value === 'standard'}
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M8 1.75L13 3.75V7.75C13 11.1 10.8 13.4 8 14.5C5.2 13.4 3 11.1 3 7.75V3.75L8 1.75Z" />
      </svg>
    {:else if value === 'accept_edits'}
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M11.3 2.7a1.5 1.5 0 0 1 2.1 2.1L5.6 12.6 2.5 13.5l.9-3.1 7.9-7.7Z" />
      </svg>
    {:else if value === 'plan'}
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="3.5" y="2.75" width="9" height="11" rx="1.2" />
        <path d="M6 2.75V2h4v.75" />
        <line x1="6" y1="6.75" x2="10" y2="6.75" />
        <line x1="6" y1="9.75" x2="9.5" y2="9.75" />
      </svg>
    {:else}
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="3.5" y="7.25" width="9" height="6.5" rx="1.3" />
        <path d="M5.5 7.25V5a2.5 2.5 0 0 1 4.7-1.2" />
        <circle cx="8" cy="10.5" r="0.8" fill="currentColor" stroke="none" />
      </svg>
    {/if}
  </button>

  {#if open}
    <div class="perm-menu" role="menu" aria-label="Permission mode">
      {#each MODES as mode (mode.id)}
        {@const selected = mode.id === value}
        <button
          type="button"
          class="perm-item"
          class:selected
          role="menuitemradio"
          aria-checked={selected}
          title={mode.description}
          onclick={() => choose(mode.id)}
        >
          <span
            class="perm-ico"
            class:tone-standard={mode.id === 'standard'}
            class:tone-accept={mode.id === 'accept_edits'}
            class:tone-plan={mode.id === 'plan'}
            class:tone-full={mode.id === 'full_access'}
            aria-hidden="true"
          >
            {#if mode.id === 'standard'}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8 1.75L13 3.75V7.75C13 11.1 10.8 13.4 8 14.5C5.2 13.4 3 11.1 3 7.75V3.75L8 1.75Z" />
              </svg>
            {:else if mode.id === 'accept_edits'}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <path d="M11.3 2.7a1.5 1.5 0 0 1 2.1 2.1L5.6 12.6 2.5 13.5l.9-3.1 7.9-7.7Z" />
              </svg>
            {:else if mode.id === 'plan'}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3.5" y="2.75" width="9" height="11" rx="1.2" />
                <path d="M6 2.75V2h4v.75" />
                <line x1="6" y1="6.75" x2="10" y2="6.75" />
                <line x1="6" y1="9.75" x2="9.5" y2="9.75" />
              </svg>
            {:else}
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3.5" y="7.25" width="9" height="6.5" rx="1.3" />
                <path d="M5.5 7.25V5a2.5 2.5 0 0 1 4.7-1.2" />
                <circle cx="8" cy="10.5" r="0.8" fill="currentColor" stroke="none" />
              </svg>
            {/if}
          </span>
          <span class="perm-label">{mode.label}</span>
          <span class="perm-info" title={mode.description} aria-hidden="true">
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3">
              <circle cx="8" cy="8" r="6" />
              <line x1="8" y1="7.2" x2="8" y2="11" stroke-linecap="round" />
              <circle cx="8" cy="5.1" r="0.6" fill="currentColor" stroke="none" />
            </svg>
          </span>
          <span class="perm-check" aria-hidden="true">
            {#if selected}
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3.2 8.4 6.5 11.7 13 4.8" />
              </svg>
            {/if}
          </span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .perm-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .perm-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    cursor: pointer;
    transition: background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .perm-trigger:hover:not(:disabled),
  .perm-trigger.active {
    background: var(--kk-surface-hover, ButtonFace);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .perm-trigger:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .tone-standard {
    color: var(--kk-text-secondary, CanvasText);
  }
  .tone-accept {
    color: var(--kk-state-success, CanvasText);
  }
  .tone-plan {
    color: var(--kk-accent, Highlight);
  }
  .tone-full {
    color: var(--kk-state-warning, CanvasText);
  }
  .perm-menu {
    position: absolute;
    left: 0;
    bottom: calc(100% + 6px);
    z-index: 30;
    min-width: 224px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-popover, none);
  }
  .perm-item {
    display: grid;
    grid-template-columns: 18px 1fr 16px 14px;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
  }
  .perm-item:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .perm-item.selected {
    background: var(--kk-surface-selected, Highlight);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .perm-ico,
  .perm-info,
  .perm-check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .perm-label {
    white-space: nowrap;
  }
  .perm-info {
    color: var(--kk-text-muted, GrayText);
  }
  .perm-check {
    color: var(--kk-text-primary, CanvasText);
  }
</style>
