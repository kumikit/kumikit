<!-- Active Git branch & worktree picker for ui-chat, built solely on @kumikit/ui-sdk and
     @kumikit/protocol. Shows a distinct worktree icon when the thread runs inside a linked
     Git worktree vs the classic branch icon in the root project folder. -->
<script lang="ts">
  import type { GitBranch, Workspace, WorkspaceGitInfo } from '@kumikit/protocol';
  import { isComposing, type Connection } from '@kumikit/ui-sdk';
  import { untrack } from 'svelte';

  let {
    connection,
    workspaceId,
    worktreePath,
    disabled = false,
    onNotify,
    onSelectWorktree,
  }: {
    connection: Connection | undefined;
    workspaceId: string;
    worktreePath?: string | undefined;
    disabled?: boolean;
    onNotify: (message: string, tone?: 'info' | 'success' | 'warning' | 'danger') => void;
    onSelectWorktree: (path: string | undefined) => void;
  } = $props();

  let open = $state(false);
  let view = $state<'menu' | 'branches' | 'worktree'>('menu');
  let git = $state.raw<WorkspaceGitInfo | undefined>();
  let workspace = $state.raw<Workspace | undefined>();
  let homeDir = $state<string | undefined>();
  let busy = $state(false);
  let branchQuery = $state('');
  let newBranch = $state('');
  let baseBranch = $state('');
  let root: HTMLElement | undefined = $state();

  let refreshSeq = 0;

  async function refresh() {
    const conn = connection;
    const wsId = workspaceId;
    const wt = worktreePath;
    if (!conn?.ready || !wsId) return;
    const seq = ++refreshSeq;
    const needWorkspace = workspace === undefined;
    try {
      const params: { workspace: string; path?: string } = { workspace: wsId };
      if (wt) params.path = wt;
      const [g, wsList] = await Promise.all([
        conn.call('workspace.git.status', params),
        needWorkspace ? conn.call('workspace.list', {}).catch(() => undefined) : Promise.resolve(undefined),
      ]);
      if (seq !== refreshSeq) return;
      git = g;
      if (wsList) {
        workspace = wsList.workspaces.find((w) => w.id === wsId);
      }
    } catch {
      // Ignore transient errors while reconnecting or on mock test connections.
    }
  }

  $effect(() => {
    void workspaceId;
    void worktreePath;
    void connection?.ready;
    git = undefined;
    workspace = undefined;
    untrack(() => void refresh());
  });

  const isWorktree = $derived(Boolean(git?.isWorktree || worktreePath));
  const displayRoot = $derived(git?.repoRoot ?? worktreePath ?? workspace?.root ?? '');
  const shortRoot = $derived.by(() => {
    if (!displayRoot) return '';
    return displayRoot.length > 26 ? `${displayRoot.slice(0, 25)}…` : displayRoot;
  });
  const triggerLabel = $derived(
    workspace?.kind === 'chat'
      ? ''
      : git?.isRepo
        ? (git.branch ?? 'HEAD')
        : (workspace?.name ?? ''),
  );
  const triggerAria = $derived(
    git?.isRepo
      ? isWorktree
        ? `Git worktree: ${triggerLabel}`
        : `Git branch: ${triggerLabel}`
      : `Workspace: ${triggerLabel}`,
  );
  const filteredBranches = $derived.by(() => {
    const list = git?.branches ?? [];
    const q = branchQuery.trim().toLowerCase();
    if (!q) return list;
    return list.filter((b) => b.name.toLowerCase().includes(q));
  });
  const worktreePreview = $derived.by(() => {
    if (!homeDir) return '';
    const base = homeDir.replace(/[\\/]+$/, '');
    const sep = base.includes('\\') && !base.includes('/') ? '\\' : '/';
    const repo = (workspace?.name ?? 'repo').replace(/[^A-Za-z0-9_.-]+/g, '-').replace(/^[-.]+|[-.]+$/g, '') || 'repo';
    const slug = (newBranch.trim() || 'new-branch').replace(/[^A-Za-z0-9_.-]+/g, '-').replace(/^[-.]+|[-.]+$/g, '') || 'worktree';
    return `${base}${sep}worktrees${sep}${repo}${sep}${slug}`;
  });

  function toggleMenu() {
    if (open) {
      open = false;
      view = 'menu';
    } else {
      open = true;
      view = 'menu';
      void refresh();
    }
  }

  function openWorktreeForm() {
    view = 'worktree';
    newBranch = '';
    baseBranch = git?.branch ?? git?.defaultBranch ?? git?.branches[0]?.name ?? '';
    if (connection?.ready) {
      connection.call('core.config.get', {}).then(
        (cfg) => {
          homeDir = cfg.homeDir;
        },
        () => {},
      );
    }
  }

  async function checkout(branch: string) {
    if (busy || !connection?.ready) return;
    busy = true;
    try {
      const params: { workspace: string; branch: string; path?: string } = {
        workspace: workspaceId,
        branch,
      };
      if (worktreePath) params.path = worktreePath;
      git = await connection.call('workspace.git.checkout', params);
      open = false;
      view = 'menu';
      onNotify(`Switched to ${branch}`, 'success');
    } catch (e) {
      onNotify(`Cannot switch branch: ${e instanceof Error ? e.message : String(e)}`, 'danger');
    } finally {
      busy = false;
    }
  }

  function pickBranch(b: GitBranch) {
    if (b.current) {
      open = false;
      view = 'menu';
      return;
    }
    if (b.worktreePath && b.worktreePath !== displayRoot) {
      open = false;
      view = 'menu';
      if (workspace?.root && b.worktreePath === workspace.root) {
        onSelectWorktree(undefined);
      } else {
        onSelectWorktree(b.worktreePath);
      }
      return;
    }
    void checkout(b.name);
  }

  async function createWorktree() {
    const branch = newBranch.trim();
    if (!branch || busy || !connection?.ready) return;
    busy = true;
    try {
      const params: { workspace: string; branch: string; baseBranch?: string } = {
        workspace: workspaceId,
        branch,
      };
      if (baseBranch) params.baseBranch = baseBranch;
      const res = await connection.call('workspace.git.worktreeCreate', params);
      open = false;
      view = 'menu';
      newBranch = '';
      onSelectWorktree(res.path);
      onNotify(`Created worktree for ${branch}`, 'success');
    } catch (e) {
      onNotify(`Cannot create worktree: ${e instanceof Error ? e.message : String(e)}`, 'danger');
    } finally {
      busy = false;
    }
  }

  async function copyPath() {
    if (!displayRoot) return;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(displayRoot);
      } else {
        const ta = document.createElement('textarea');
        ta.value = displayRoot;
        ta.style.position = 'fixed';
        ta.style.opacity = '0';
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        ta.remove();
      }
      open = false;
      view = 'menu';
      onNotify('Copied project path', 'info');
    } catch {
      onNotify('Cannot copy path to clipboard', 'warning');
    }
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (!open) return;
    if (root && !root.contains(e.target as Node)) {
      open = false;
      view = 'menu';
    }
  }

  function onWindowKeyDown(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (open && e.key === 'Escape') {
      e.preventDefault();
      if (view !== 'menu') {
        view = 'menu';
      } else {
        open = false;
      }
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeyDown} />

{#if triggerLabel}
  <div class="branch-wrap" bind:this={root}>
    <button
      type="button"
      class="branch-trigger"
      class:active={open}
      class:is-worktree={isWorktree}
      aria-label={triggerAria}
      aria-haspopup="menu"
      aria-expanded={open}
      title={displayRoot
        ? `${isWorktree ? 'Worktree' : 'Project folder'}: ${triggerLabel} · ${displayRoot}`
        : triggerLabel}
      disabled={disabled || !connection?.ready}
      onclick={toggleMenu}
    >
      {#if isWorktree}
        <svg class="wt-ico" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="2" y="2.25" width="4.75" height="4.75" rx="1" />
          <rect x="9.25" y="9" width="4.75" height="4.75" rx="1" />
          <path d="M6.75 4.6h2.75a2 2 0 0 1 2 2V9" />
        </svg>
      {:else if git?.isRepo}
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="5" cy="3.5" r="1.5" />
          <circle cx="5" cy="12.5" r="1.5" />
          <circle cx="11.5" cy="6" r="1.5" />
          <path d="M5 5v6" />
          <path d="M11.5 7.5c0 2-1.2 3-3.5 3H5" />
        </svg>
      {:else}
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M2.5 4.25h4l1.5 1.5h5.5v6.75h-11Z" />
        </svg>
      {/if}
      <span class="branch-name">{triggerLabel}</span>
    </button>

    {#if open}
      <div class="branch-menu" role="menu" aria-label="Branch and workspace actions">
        {#if view === 'menu'}
          <div class="menu-head" title={displayRoot}>
            <span class="menu-ico" class:wt-ico={isWorktree} aria-hidden="true">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                {#if isWorktree}
                  <rect x="2" y="2.25" width="4.75" height="4.75" rx="1" />
                  <rect x="9.25" y="9" width="4.75" height="4.75" rx="1" />
                  <path d="M6.75 4.6h2.75a2 2 0 0 1 2 2V9" />
                {:else if git?.isRepo}
                  <circle cx="5" cy="3.5" r="1.5" />
                  <circle cx="5" cy="12.5" r="1.5" />
                  <circle cx="11.5" cy="6" r="1.5" />
                  <path d="M5 5v6" />
                  <path d="M11.5 7.5c0 2-1.2 3-3.5 3H5" />
                {:else}
                  <path d="M2.5 4.25h4l1.5 1.5h5.5v6.75h-11Z" />
                {/if}
              </svg>
            </span>
            <div class="head-text">
              <span class="head-title">{triggerLabel}</span>
              <span class="head-sub">{isWorktree ? 'Worktree' : 'Local'}{shortRoot ? ` · ${shortRoot}` : ''}</span>
            </div>
            <span class="menu-check" aria-hidden="true">
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3.2 8.4 6.5 11.7 13 4.8" />
              </svg>
            </span>
          </div>

          {#if git?.isRepo}
            {#if !isWorktree && git.defaultBranch && git.branch !== git.defaultBranch}
              <button
                type="button"
                class="menu-item"
                role="menuitem"
                disabled={busy}
                onclick={() => void checkout(git!.defaultBranch!)}
              >
                <span class="menu-ico" aria-hidden="true">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="5" cy="3.5" r="1.5" />
                    <circle cx="5" cy="12.5" r="1.5" />
                    <circle cx="11.5" cy="6" r="1.5" />
                    <path d="M5 5v6" />
                    <path d="M11.5 7.5c0 2-1.2 3-3.5 3H5" />
                  </svg>
                </span>
                <span class="item-label">Switch to {git.defaultBranch}</span>
              </button>
            {/if}

            <button
              type="button"
              class="menu-item"
              role="menuitem"
              disabled={busy || git.branches.length === 0}
              onclick={() => {
                branchQuery = '';
                view = 'branches';
              }}
            >
              <span class="menu-ico" aria-hidden="true">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="5" cy="3.5" r="1.5" />
                  <circle cx="5" cy="12.5" r="1.5" />
                  <circle cx="11.5" cy="6" r="1.5" />
                  <path d="M5 5v6" />
                  <path d="M11.5 7.5c0 2-1.2 3-3.5 3H5" />
                </svg>
              </span>
              <span class="item-label">Other branch…</span>
              <span class="item-meta">
                {git.branches.length} local {git.branches.length === 1 ? 'branch' : 'branches'}
              </span>
            </button>

            <button
              type="button"
              class="menu-item"
              role="menuitem"
              disabled={busy}
              onclick={openWorktreeForm}
            >
              <span class="menu-ico" aria-hidden="true">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="2.25" width="4.75" height="4.75" rx="1" />
                  <rect x="9.25" y="9" width="4.75" height="4.75" rx="1" />
                  <path d="M6.75 4.6h2.75a2 2 0 0 1 2 2V9" />
                </svg>
              </span>
              <span class="item-label">New thread in a worktree…</span>
            </button>

            <div class="menu-sep" role="separator"></div>
          {/if}

          <button
            type="button"
            class="menu-item"
            role="menuitem"
            disabled={!displayRoot}
            onclick={() => void copyPath()}
          >
            <span class="menu-ico" aria-hidden="true">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <rect x="5.5" y="5.5" width="8" height="8" rx="1.2" />
                <path d="M10.5 5.5V3.7a1.2 1.2 0 0 0-1.2-1.2H3.7a1.2 1.2 0 0 0-1.2 1.2v5.6a1.2 1.2 0 0 0 1.2 1.2h1.8" />
              </svg>
            </span>
            <span class="item-label">Copy project path</span>
          </button>
        {:else if view === 'branches'}
          <div class="sub-head">
            <button type="button" class="back-btn" onclick={() => (view = 'menu')}>← Back</button>
            <span class="sub-title">Local branches</span>
          </div>
          <div class="sub-filter">
            <label class="kk-sr-only" for="kk-chat-branch-filter">Filter branches</label>
            <input
              id="kk-chat-branch-filter"
              type="text"
              class="sub-input"
              placeholder="Filter branches…"
              bind:value={branchQuery}
            />
          </div>
          <div class="branch-list">
            {#if filteredBranches.length === 0}
              <p class="empty-msg">No matching branches.</p>
            {:else}
              {#each filteredBranches as b (b.name)}
                <button
                  type="button"
                  class="menu-item"
                  class:selected={b.current}
                  role="menuitem"
                  disabled={busy}
                  title={b.worktreePath ? `Checked out at ${b.worktreePath}` : `Switch to ${b.name}`}
                  onclick={() => pickBranch(b)}
                >
                  <span class="menu-ico" aria-hidden="true">
                    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                      <circle cx="5" cy="3.5" r="1.5" />
                      <circle cx="5" cy="12.5" r="1.5" />
                      <circle cx="11.5" cy="6" r="1.5" />
                      <path d="M5 5v6" />
                      <path d="M11.5 7.5c0 2-1.2 3-3.5 3H5" />
                    </svg>
                  </span>
                  <span class="item-label">{b.name}</span>
                  {#if b.current}
                    <span class="menu-check" aria-hidden="true">✓</span>
                  {:else if b.worktreePath}
                    <span class="item-meta">worktree</span>
                  {/if}
                </button>
              {/each}
            {/if}
          </div>
        {:else}
          <div class="sub-head">
            <button type="button" class="back-btn" onclick={() => (view = 'menu')}>← Back</button>
            <span class="sub-title">New thread in a worktree</span>
          </div>
          <div class="wt-form">
            <label class="wt-field">
              <span>Branch name</span>
              <input
                type="text"
                class="sub-input"
                placeholder="feat/my-change"
                bind:value={newBranch}
                disabled={busy}
                onkeydown={(e) => {
                  if (isComposing(e)) return;
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    void createWorktree();
                  }
                }}
              />
            </label>
            <label class="wt-field">
              <span>From branch</span>
              <select class="sub-input" bind:value={baseBranch} disabled={busy}>
                {#each git?.branches ?? [] as b (b.name)}
                  <option value={b.name}>{b.name}</option>
                {/each}
              </select>
            </label>
            {#if worktreePreview}
              <p class="wt-preview" title={worktreePreview}>{worktreePreview}</p>
            {/if}
            <button
              type="button"
              class="wt-submit"
              disabled={busy || !newBranch.trim()}
              onclick={() => void createWorktree()}
            >
              {busy ? 'Creating…' : 'Create worktree & start thread'}
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .branch-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-width: 0;
  }
  .branch-trigger {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    max-width: 220px;
    padding: 0 7px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
    transition: background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .branch-trigger.is-worktree {
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .wt-ico {
    color: var(--kk-accent, Highlight);
  }
  .branch-trigger:hover:not(:disabled),
  .branch-trigger.active {
    background: var(--kk-surface-hover, ButtonFace);
    border-color: var(--kk-border-subtle, ButtonBorder);
    color: var(--kk-text-primary, CanvasText);
  }
  .branch-trigger:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .branch-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .branch-menu {
    position: absolute;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 35;
    width: 292px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-popover, none);
  }
  .menu-head {
    display: grid;
    grid-template-columns: 18px 1fr 14px;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    font-size: var(--kk-font-size-small, small);
  }
  .head-text {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .head-title {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    white-space: nowrap;
  }
  .head-sub {
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-item {
    display: flex;
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
  .menu-item:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .menu-item.selected {
    background: var(--kk-surface-selected, Highlight);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .menu-item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .menu-ico,
  .menu-check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    flex: none;
    color: var(--kk-text-secondary, CanvasText);
  }
  .menu-ico.wt-ico {
    color: var(--kk-accent, Highlight);
  }
  .item-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-meta {
    color: var(--kk-text-muted, GrayText);
    white-space: nowrap;
    flex: none;
  }
  .menu-sep {
    height: 1px;
    margin: 3px 6px;
    background: var(--kk-border-subtle, CanvasText);
  }
  .sub-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .back-btn {
    padding: 3px 6px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .back-btn:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .sub-title {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .sub-filter {
    padding: 6px 6px 2px;
  }
  .sub-input {
    width: 100%;
    padding: 6px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font-size: var(--kk-font-size-small, small);
  }
  .sub-input:focus {
    outline: none;
    border-color: var(--kk-border-strong, Highlight);
  }
  .branch-list {
    max-height: 210px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: 2px;
  }
  .empty-msg {
    margin: 0;
    padding: 10px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    text-align: center;
  }
  .wt-form {
    display: grid;
    gap: 8px;
    padding: 8px 6px 6px;
  }
  .wt-field {
    display: grid;
    gap: 4px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .wt-preview {
    margin: 0;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wt-submit {
    padding: 7px 10px;
    border: 0;
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    cursor: pointer;
  }
  .wt-submit:hover:not(:disabled) {
    background: var(--kk-accent-hover, Highlight);
  }
  .wt-submit:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
