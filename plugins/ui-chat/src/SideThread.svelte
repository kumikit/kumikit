<!-- Inspector panel hosting multiple side threads (`/btw`) linked to the active main
     thread. Tracks running and completed side threads for that main thread so you can
     switch between them or start new ones without interrupting the main turn. -->
<script lang="ts">
  import type { Thread } from '@kumikit/protocol';
  import type { Connection, PluginContext, ThreadAddress } from '@kumikit/ui-sdk';
  import Chat from './Chat.svelte';

  let {
    ctx,
    address,
    connection,
  }: {
    ctx: PluginContext;
    address?: ThreadAddress;
    connection?: Connection;
  } = $props();

  const conn = $derived(connection ?? ctx.connection);
  let sideThreads = $state.raw<Thread[]>([]);
  let selectedId = $state<string | undefined>();
  let composingNew = $state(false);
  let loading = $state(false);
  let draft = $state('');

  const selectedThread = $derived(sideThreads.find((t) => t.id === selectedId));
  const sideAddress = $derived<ThreadAddress | undefined>(
    address && !composingNew && selectedId
      ? { connection: address.connection, workspace: address.workspace, thread: selectedId }
      : undefined,
  );

  function sortSides(list: Thread[]): Thread[] {
    return [...list].sort((a, b) => (a.created < b.created ? 1 : a.created > b.created ? -1 : 0));
  }

  function mergeSide(next: Thread) {
    if (!address || !next.side || next.parent !== address.thread) return;
    const prev = sideThreads.find((t) => t.id === next.id);
    if (prev && prev.updated > next.updated) return;
    sideThreads = sortSides([next, ...sideThreads.filter((t) => t.id !== next.id)]);
    // A thread that arrives while none is shown (one an agent just started) is shown at
    // once, as a loaded one is; one the user is writing is not interrupted.
    if (!selectedId && !composingNew) selectedId = next.id;
  }

  function removeSide(id: string) {
    const next = sideThreads.filter((t) => t.id !== id);
    if (next.length === sideThreads.length) return;
    sideThreads = next;
    if (selectedId === id) {
      selectedId = next[0]?.id;
    }
  }

  function statusTag(status: Thread['status']): string {
    switch (status) {
      case 'running':
        return 'running';
      case 'awaiting_approval':
        return 'waiting';
      case 'limited':
        return 'limited';
      case 'failed':
        return 'failed';
      default:
        return '';
    }
  }

  function labelFor(t: Thread): string {
    const base = t.title?.trim() || 'Side thread';
    const st = statusTag(t.status);
    return st ? `${base} (${st})` : base;
  }

  // Load all side threads for the active main thread and stay synced via the workspace topic.
  $effect(() => {
    const parent = address;
    const c = conn;
    if (!parent || !c?.ready) {
      sideThreads = [];
      selectedId = undefined;
      composingNew = false;
      return;
    }
    let alive = true;
    loading = true;
    composingNew = false;
    c.call('thread.list', { workspace: parent.workspace, includeSide: true }).then(
      (res) => {
        if (!alive) return;
        const matches = sortSides((res?.threads ?? []).filter((t) => t.side && t.parent === parent.thread));
        sideThreads = matches;
        if (!selectedId || !matches.some((t) => t.id === selectedId)) {
          selectedId = matches[0]?.id;
        }
        loading = false;
      },
      () => {
        if (!alive) return;
        sideThreads = [];
        selectedId = undefined;
        loading = false;
      },
    );
    const stop = c.subscribe(`workspace:${parent.workspace}`, 0, (ev) => {
      if (!alive) return;
      if (ev.kind === 'thread.created' || ev.kind === 'thread.updated' || ev.kind === 'thread.archived') {
        mergeSide(ev.data.thread);
      } else if (ev.kind === 'thread.deleted') {
        removeSide(ev.data.thread.id);
      }
    });
    return () => {
      alive = false;
      stop();
    };
  });

  // React to `/btw` creating or focusing a side thread while this panel is open.
  $effect(() => {
    if (typeof window === 'undefined') return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent<{ parent: ThreadAddress; side: string; thread?: Thread }>).detail;
      if (
        detail &&
        address &&
        detail.parent.connection === address.connection &&
        detail.parent.workspace === address.workspace &&
        detail.parent.thread === address.thread
      ) {
        if (detail.thread) mergeSide(detail.thread);
        selectedId = detail.side;
        composingNew = false;
      }
    };
    window.addEventListener('kumikit:side-thread-opened', handler);
    return () => window.removeEventListener('kumikit:side-thread-opened', handler);
  });

  async function startSide() {
    const q = draft.trim();
    const parent = address;
    const c = conn;
    if (!q || !parent || !c?.ready) return;
    draft = '';
    const title = q.length > 48 ? `${q.slice(0, 45)}…` : q;
    const { thread } = await c.call('thread.create', {
      workspace: parent.workspace,
      parent: parent.thread,
      side: true,
      title,
    });
    mergeSide(thread);
    selectedId = thread.id;
    composingNew = false;
    const nextAddr: ThreadAddress = {
      connection: parent.connection,
      workspace: parent.workspace,
      thread: thread.id,
    };
    await ctx.commands.run('thread.send', { address: nextAddr, text: q });
  }
</script>

<div class="kk-side">
  {#if !address}
    <p class="side-empty">Open a thread to ask side questions with <code>/btw</code>.</p>
  {:else}
    <div class="side-bar" role="region" aria-label="Side threads">
      {#if sideThreads.length > 0}
        <span class="side-dot" data-status={selectedThread?.status ?? 'idle'} aria-hidden="true"></span>
        <select
          class="side-select"
          aria-label="Side thread history"
          value={composingNew ? '__new__' : (selectedId ?? '')}
          onchange={(e) => {
            const val = (e.currentTarget as HTMLSelectElement).value;
            if (val === '__new__') {
              composingNew = true;
            } else {
              selectedId = val;
              composingNew = false;
            }
          }}
        >
          {#if composingNew}
            <option value="__new__">New side thread…</option>
          {/if}
          {#each sideThreads as t (t.id)}
            <option value={t.id}>{labelFor(t)}</option>
          {/each}
        </select>
        <span class="side-count" title={`${sideThreads.length} side ${sideThreads.length === 1 ? 'thread' : 'threads'}`}>
          {sideThreads.length}
        </span>
      {:else}
        <span class="side-bar-label">Side threads</span>
      {/if}
      <button
        type="button"
        class="side-icon-btn"
        aria-label="New side thread"
        title="Start a new side thread"
        aria-pressed={composingNew}
        onclick={() => {
          if (composingNew && sideThreads.length > 0) {
            composingNew = false;
            if (!selectedId) selectedId = sideThreads[0]?.id;
          } else {
            composingNew = true;
          }
        }}
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
          <line x1="8" y1="3" x2="8" y2="13" />
          <line x1="3" y1="8" x2="13" y2="8" />
        </svg>
      </button>
    </div>

    {#if sideAddress}
      {#key sideAddress.thread}
        <div class="side-chat-wrap">
          <Chat {ctx} address={sideAddress} {connection} sideFor={address} />
        </div>
      {/key}
    {:else if loading && sideThreads.length === 0}
      <p class="side-empty">Loading side threads…</p>
    {:else}
      <div class="side-start">
        <p class="side-title">New side thread</p>
        <p class="side-note">Ask a quick question with context from the main thread. It never interrupts or queues into the main turn.</p>
        <form
          class="side-form"
          onsubmit={(e) => {
            e.preventDefault();
            void startSide();
          }}
        >
          <input
            type="text"
            class="side-input"
            placeholder="Ask a side question…"
            aria-label="Side question"
            bind:value={draft}
          />
          <button type="submit" class="side-btn" disabled={!draft.trim()}>Ask</button>
        </form>

        {#if sideThreads.length > 0}
          <div class="side-history">
            <p class="side-history-title">History ({sideThreads.length})</p>
            <ul class="side-history-list" aria-label="Side thread history list">
              {#each sideThreads as t (t.id)}
                <li>
                  <button
                    type="button"
                    class="side-history-item"
                    onclick={() => {
                      selectedId = t.id;
                      composingNew = false;
                    }}
                  >
                    <span class="side-dot" data-status={t.status} aria-hidden="true"></span>
                    <span class="side-history-name">{t.title || 'Side thread'}</span>
                    {#if statusTag(t.status)}
                      <span class="side-history-status">{statusTag(t.status)}</span>
                    {/if}
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .kk-side {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .side-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-frame, Canvas);
    flex: none;
  }
  .side-bar-label {
    flex: 1;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .side-select {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .side-count {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    flex: none;
  }
  .side-icon-btn {
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
    flex: none;
  }
  .side-icon-btn:hover,
  .side-icon-btn[aria-pressed='true'] {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .side-dot {
    width: 7px;
    height: 7px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-text-muted, GrayText);
    flex: none;
  }
  .side-dot[data-status='running'] {
    background: var(--kk-accent, Highlight);
  }
  .side-dot[data-status='awaiting_approval'],
  .side-dot[data-status='limited'] {
    background: var(--kk-state-warning, Mark);
  }
  .side-dot[data-status='failed'] {
    background: var(--kk-state-danger, CanvasText);
  }
  .side-chat-wrap {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .side-empty {
    margin: 0;
    padding: 16px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .side-start {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow: auto;
  }
  .side-title {
    margin: 0;
    font-size: var(--kk-font-size-body, medium);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .side-note {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    line-height: 1.5;
  }
  .side-form {
    display: flex;
    gap: 6px;
  }
  .side-input {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
  }
  .side-btn {
    padding: 6px 12px;
    border: 1px solid var(--kk-accent, Highlight);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .side-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .side-history {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .side-history-title {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-muted, GrayText);
  }
  .side-history-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .side-history-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
  }
  .side-history-item:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .side-history-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .side-history-status {
    color: var(--kk-text-muted, GrayText);
    flex: none;
  }
</style>
