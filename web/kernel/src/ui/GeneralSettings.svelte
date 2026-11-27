<!-- General settings: default agent permission mode for new threads, and the active
     core's Kumikit home folder (where worktrees and other Kumikit-created directories live). -->
<script lang="ts">
  import type { PermissionMode } from '@kumikit/protocol';
  import type { Kernel } from '../app.svelte';
  import type { WindowButtonsMode } from '../settings/settings.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  interface WindowButtonsOption {
    id: WindowButtonsMode;
    label: string;
    description: string;
  }

  const windowButtonsMode = $derived(kernel.settings.value.windowButtons);
  const windowButtonsOptions = $derived.by<WindowButtonsOption[]>(() => {
    const detected = kernel.windowChrome?.tiling ? 'tiling window manager' : 'stacking desktop';
    return [
      {
        id: 'auto',
        label: 'Auto',
        description: `Show on stacking desktops and hide on tiling window managers (detected: ${detected}).`,
      },
      {
        id: 'show',
        label: 'Show',
        description: 'Always show minimize, maximize, and close buttons in the top bar.',
      },
      {
        id: 'hide',
        label: 'Hide',
        description: 'Never show window buttons in the top bar.',
      },
    ];
  });

  function setWindowButtons(mode: WindowButtonsMode) {
    kernel.settings.update({ windowButtons: mode });
  }

  interface ModeOption {
    id: PermissionMode;
    label: string;
    description: string;
  }

  const MODES: ModeOption[] = [
    {
      id: 'standard',
      label: 'Standard',
      description: 'Ask before running commands or editing files.',
    },
    {
      id: 'accept_edits',
      label: 'Accept edits',
      description: 'Auto-approve workspace edits; ask before running commands.',
    },
    {
      id: 'plan',
      label: 'Plan mode',
      description: 'Read-only planning and exploration before making changes.',
    },
    {
      id: 'full_access',
      label: 'Full access',
      description: 'Allow edits and commands without asking.',
    },
  ];

  const conn = $derived(kernel.connection);
  const defaultMode = $derived(kernel.settings.value.defaultPermissionMode);

  let homeInput = $state('');
  let savedHome = $state('');
  let defaultHome = $state('');
  let loadingHome = $state(false);
  let savingHome = $state(false);
  let homeError = $state<string | null>(null);

  $effect(() => {
    const c = conn;
    const ready = c.ready;
    if (!ready) return;
    let alive = true;
    loadingHome = true;
    homeError = null;
    c.call('core.config.get', {}).then(
      (res) => {
        if (!alive) return;
        loadingHome = false;
        savedHome = res.homeDir;
        defaultHome = res.defaultHomeDir;
        homeInput = res.homeDir;
      },
      (e) => {
        if (!alive) return;
        loadingHome = false;
        homeError = e instanceof Error ? e.message : String(e);
      },
    );
    return () => {
      alive = false;
    };
  });

  const dirtyHome = $derived(homeInput.trim() !== savedHome);
  const isCustomHome = $derived(Boolean(savedHome && defaultHome && savedHome !== defaultHome));
  const worktreesPreview = $derived.by(() => {
    const base = (homeInput.trim() || savedHome || defaultHome).replace(/[\\/]+$/, '');
    if (!base) return '';
    const sep = base.includes('\\') && !base.includes('/') ? '\\' : '/';
    return `${base}${sep}worktrees`;
  });

  function setDefaultMode(mode: PermissionMode) {
    kernel.settings.update({ defaultPermissionMode: mode });
  }

  async function browseHome() {
    if (conn.kind !== 'local') return;
    const picked = await kernel.platform.pickFolder('Choose Kumikit home folder');
    if (picked) {
      homeInput = picked;
      await saveHome();
    }
  }

  async function saveHome() {
    const next = homeInput.trim();
    savingHome = true;
    homeError = null;
    try {
      const res = await conn.call('core.config.set', next ? { homeDir: next } : {});
      savedHome = res.homeDir;
      defaultHome = res.defaultHomeDir;
      homeInput = res.homeDir;
      kernel.notify('Updated Kumikit home folder', 'success');
    } catch (e) {
      homeError = e instanceof Error ? e.message : String(e);
    } finally {
      savingHome = false;
    }
  }

  async function resetHome() {
    savingHome = true;
    homeError = null;
    try {
      const res = await conn.call('core.config.set', {});
      savedHome = res.homeDir;
      defaultHome = res.defaultHomeDir;
      homeInput = res.homeDir;
      kernel.notify('Reset Kumikit home folder to default', 'info');
    } catch (e) {
      homeError = e instanceof Error ? e.message : String(e);
    } finally {
      savingHome = false;
    }
  }
</script>

<div class="general">
  <div class="intro">
    <h2>General</h2>
    <p class="kk-muted">Default thread permissions and storage folders for Kumikit.</p>
  </div>

  <fieldset class="section">
    <legend>Default permission mode</legend>
    <p class="hint">Applied when opening a new thread. You can still change permissions per thread in the composer.</p>
    <div class="options">
      {#each MODES as m (m.id)}
        <label class="option" class:selected={defaultMode === m.id}>
          <input
            type="radio"
            name="kk-default-permission"
            value={m.id}
            checked={defaultMode === m.id}
            onchange={() => setDefaultMode(m.id)}
          />
          <div class="opt-body">
            <span class="opt-title">{m.label}</span>
            <span class="opt-desc">{m.description}</span>
          </div>
          {#if m.id === 'standard'}
            <span class="chip">Default</span>
          {/if}
        </label>
      {/each}
    </div>
  </fieldset>

  <section class="section" aria-labelledby="kk-home-heading">
    <h3 id="kk-home-heading">Kumikit home folder</h3>
    <p class="hint">
      Where Kumikit stores files and directories it creates on <strong>{conn.label}</strong>, such as Git worktrees.
    </p>
    <form
      class="home-row"
      onsubmit={(e) => {
        e.preventDefault();
        void saveHome();
      }}
    >
      <label class="kk-sr-only" for="kk-home-input">Kumikit home folder</label>
      <input
        id="kk-home-input"
        class="home-input"
        type="text"
        bind:value={homeInput}
        placeholder={defaultHome || '~/.kumikit'}
        disabled={!conn.ready || loadingHome || savingHome}
      />
      {#if conn.kind === 'local'}
        <button
          type="button"
          class="kk-button"
          disabled={!conn.ready || loadingHome || savingHome}
          onclick={() => void browseHome()}
        >
          Browse…
        </button>
      {/if}
      <button
        type="submit"
        class="kk-button primary"
        disabled={!conn.ready || loadingHome || savingHome || !dirtyHome}
      >
        Save
      </button>
      {#if isCustomHome}
        <button
          type="button"
          class="kk-button"
          disabled={!conn.ready || savingHome}
          onclick={() => void resetHome()}
        >
          Reset
        </button>
      {/if}
    </form>
    {#if worktreesPreview}
      <div class="path-row">
        <p class="path-note">Worktrees folder: <code>{worktreesPreview}</code></p>
        <button
          type="button"
          class="link-btn"
          onclick={() => (kernel.main = { kind: 'settings', page: 'worktrees' })}
        >
          Manage worktrees
        </button>
      </div>
    {/if}
    {#if homeError}
      <p class="err" role="alert">{homeError}</p>
    {/if}
  </section>

  {#if kernel.platform.window && kernel.windowChrome?.borderless}
    <fieldset class="section">
      <legend>Window buttons</legend>
      <p class="hint">Control minimize, maximize, and close buttons in the top bar.</p>
      <div class="options">
        {#each windowButtonsOptions as opt (opt.id)}
          <label class="option" class:selected={windowButtonsMode === opt.id}>
            <input
              type="radio"
              name="kk-window-buttons"
              value={opt.id}
              checked={windowButtonsMode === opt.id}
              onchange={() => setWindowButtons(opt.id)}
            />
            <div class="opt-body">
              <span class="opt-title">{opt.label}</span>
              <span class="opt-desc">{opt.description}</span>
            </div>
            {#if opt.id === 'auto'}
              <span class="chip">Default</span>
            {/if}
          </label>
        {/each}
      </div>
    </fieldset>
  {/if}
</div>

<style>
  .general {
    display: grid;
    gap: 24px;
    max-width: 600px;
  }
  .intro {
    display: grid;
    gap: 4px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .intro p,
  .hint {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .section {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  legend,
  h3 {
    padding: 0;
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .options {
    display: grid;
    gap: 6px;
    margin-top: 2px;
  }
  .option {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    cursor: pointer;
    font-size: var(--kk-font-size-small, small);
  }
  .option:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .option.selected {
    border-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-selected, Highlight);
  }
  .option input {
    margin-top: 2px;
  }
  .opt-body {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .opt-title {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .opt-desc {
    color: var(--kk-text-secondary, CanvasText);
  }
  .chip {
    margin-left: auto;
    padding: 1px 7px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    flex: none;
  }
  .home-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
  }
  .home-input {
    flex: 1;
    min-width: 0;
    padding: 7px 10px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
  }
  .home-input:focus {
    outline: none;
    border-color: var(--kk-border-strong, Highlight);
  }
  .path-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .path-note {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    word-break: break-all;
  }
  .path-note code {
    font-family: var(--kk-font-mono, monospace);
    color: var(--kk-text-secondary, CanvasText);
  }
  .link-btn {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--kk-accent, Highlight);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
    text-decoration: underline;
    flex: none;
  }
  .link-btn:hover {
    color: var(--kk-text-primary, CanvasText);
  }
  .err {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-state-danger, CanvasText);
  }
</style>
