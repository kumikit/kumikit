<!-- Minimal 16px stroke SVG icons (`currentColor`). Purely decorative; callers provide
     accessible labels and tooltips. -->
<script lang="ts">
  export type IconName =
    | 'sidebar-left'
    | 'sidebar-right'
    | 'split-h'
    | 'split-v'
    | 'folder-plus'
    | 'folder'
    | 'plus'
    | 'message-plus'
    | 'chevron-right'
    | 'chevron-down'
    | 'settings'
    | 'plugins'
    | 'server'
    | 'command'
    | 'sun'
    | 'moon'
    | 'x'
    | 'stop'
    | 'layers'
    | 'search'
    | 'arrow-left'
    | 'panel';

  let { name, size = 14 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.4"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'sidebar-left'}
    <rect x="1.75" y="2.25" width="12.5" height="11.5" rx="1.75" />
    <line x1="6" y1="2.25" x2="6" y2="13.75" />
  {:else if name === 'sidebar-right'}
    <rect x="1.75" y="2.25" width="12.5" height="11.5" rx="1.75" />
    <line x1="10" y1="2.25" x2="10" y2="13.75" />
  {:else if name === 'split-h'}
    <rect x="1.75" y="2.25" width="12.5" height="11.5" rx="1.75" />
    <line x1="8" y1="2.25" x2="8" y2="13.75" />
  {:else if name === 'split-v'}
    <rect x="1.75" y="2.25" width="12.5" height="11.5" rx="1.75" />
    <line x1="1.75" y1="8" x2="14.25" y2="8" />
  {:else if name === 'folder-plus'}
    <path d="M1.75 4.25A1.25 1.25 0 0 1 3 3h2.8l1.3 1.5H13A1.25 1.25 0 0 1 14.25 5.75v6A1.25 1.25 0 0 1 13 13H3a1.25 1.25 0 0 1-1.25-1.25z" />
    <line x1="8" y1="7" x2="8" y2="10.5" />
    <line x1="6.25" y1="8.75" x2="9.75" y2="8.75" />
  {:else if name === 'folder'}
    <path d="M1.75 4.25A1.25 1.25 0 0 1 3 3h2.8l1.3 1.5H13A1.25 1.25 0 0 1 14.25 5.75v6A1.25 1.25 0 0 1 13 13H3a1.25 1.25 0 0 1-1.25-1.25z" />
  {:else if name === 'plus'}
    <line x1="8" y1="3" x2="8" y2="13" />
    <line x1="3" y1="8" x2="13" y2="8" />
  {:else if name === 'message-plus'}
    <path d="M2.25 3.75A1.5 1.5 0 0 1 3.75 2.25h8.5a1.5 1.5 0 0 1 1.5 1.5v6.5a1.5 1.5 0 0 1-1.5 1.5H6.25L3 14v-2.25H3.75a1.5 1.5 0 0 1-1.5-1.5z" />
    <line x1="8" y1="5" x2="8" y2="9" />
    <line x1="6" y1="7" x2="10" y2="7" />
  {:else if name === 'chevron-right'}
    <polyline points="6 4 10 8 6 12" />
  {:else if name === 'chevron-down'}
    <polyline points="4 6 8 10 12 6" />
  {:else if name === 'settings'}
    <line x1="2.5" y1="4.5" x2="13.5" y2="4.5" />
    <circle cx="5.5" cy="4.5" r="1.4" fill="var(--kk-surface-frame, Canvas)" />
    <line x1="2.5" y1="8" x2="13.5" y2="8" />
    <circle cx="10.5" cy="8" r="1.4" fill="var(--kk-surface-frame, Canvas)" />
    <line x1="2.5" y1="11.5" x2="13.5" y2="11.5" />
    <circle cx="6.5" cy="11.5" r="1.4" fill="var(--kk-surface-frame, Canvas)" />
  {:else if name === 'plugins'}
    <path d="M5.5 2.5h5v2.25a1.25 1.25 0 1 1 0 2.5V9.5h2.25a1.25 1.25 0 1 1 0 2.5v1.5h-7.25v-2.25a1.25 1.25 0 1 0 0-2.5V2.5z" />
  {:else if name === 'server'}
    <rect x="2" y="2.5" width="12" height="4.25" rx="1" />
    <rect x="2" y="9.25" width="12" height="4.25" rx="1" />
    <line x1="4.5" y1="4.6" x2="4.55" y2="4.6" />
    <line x1="4.5" y1="11.35" x2="4.55" y2="11.35" />
  {:else if name === 'command'}
    <path d="M5.5 5.5v5M10.5 5.5v5M5.5 5.5h5M5.5 10.5h5M5.5 5.5a1.75 1.75 0 1 1 0-3.5 1.75 1.75 0 0 1 0 3.5zM10.5 5.5a1.75 1.75 0 1 0 0-3.5 1.75 1.75 0 0 0 0 3.5zM5.5 10.5a1.75 1.75 0 1 0 0 3.5 1.75 1.75 0 0 0 0-3.5zM10.5 10.5a1.75 1.75 0 1 1 0 3.5 1.75 1.75 0 0 1 0-3.5z" />
  {:else if name === 'sun'}
    <circle cx="8" cy="8" r="2.75" />
    <path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1" />
  {:else if name === 'moon'}
    <path d="M12.75 9.75A5 5 0 0 1 6.25 3.25a5.25 5.25 0 1 0 6.5 6.5z" />
  {:else if name === 'x'}
    <line x1="4" y1="4" x2="12" y2="12" />
    <line x1="12" y1="4" x2="4" y2="12" />
  {:else if name === 'stop'}
    <rect x="4" y="4" width="8" height="8" rx="1" fill="currentColor" stroke="none" />
  {:else if name === 'layers'}
    <polygon points="8 2 14 5.5 8 9 2 5.5 8 2" />
    <polyline points="2 8.5 8 12 14 8.5" />
    <polyline points="2 11.25 8 14.5 14 11.25" />
  {:else if name === 'search'}
    <circle cx="7" cy="7" r="4.25" />
    <line x1="10.25" y1="10.25" x2="13.75" y2="13.75" />
  {:else if name === 'arrow-left'}
    <line x1="12.5" y1="8" x2="3.5" y2="8" />
    <polyline points="7.5 4 3.5 8 7.5 12" />
  {:else if name === 'panel'}
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <line x1="2" y1="6" x2="14" y2="6" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    flex: none;
  }
</style>
