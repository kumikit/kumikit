<!-- Inspector panel hosting a side thread (`/btw`) linked to the active thread.
     Never interrupts or queues into the main thread; answers offer Insert into main. -->
<script lang="ts">
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
  let sideId = $state<string | undefined>();
  let loading = $state(false);
  let draft = $state('');
  let resolvedFor = $state('');

  const sideAddress = $derived<ThreadAddress | undefined>(
    address && sideId ? { connection: address.connection, workspace: address.workspace, thread: sideId } : undefined,
  );

  // Look up an existing side thread for the active main thread whenever it changes.
  $effect(() => {
    const parent = address;
    const c = conn;
    if (!parent || !c?.ready) {
      sideId = undefined;
      resolvedFor = '';
      return;
    }
    const key = `${parent.connection}/${parent.workspace}/${parent.thread}`;
    if (key === resolvedFor && sideId) return;
    let alive = true;
    loading = true;
    c.call('thread.list', { workspace: parent.workspace, includeSide: true }).then(
      (res) => {
        if (!alive) return;
        const match = (res?.threads ?? []).find((t) => t.side && t.parent === parent.thread);
        sideId = match?.id;
        resolvedFor = key;
        loading = false;
      },
      () => {
        if (!alive) return;
        sideId = undefined;
        resolvedFor = key;
        loading = false;
      },
    );
    return () => {
      alive = false;
    };
  });

  // React to `/btw` creating a new side thread while this panel is open.
  $effect(() => {
    if (typeof window === 'undefined') return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent<{ parent: ThreadAddress; side: string }>).detail;
      if (
        detail &&
        address &&
        detail.parent.connection === address.connection &&
        detail.parent.workspace === address.workspace &&
        detail.parent.thread === address.thread
      ) {
        sideId = detail.side;
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
    const { thread } = await c.call('thread.create', {
      workspace: parent.workspace,
      parent: parent.thread,
      side: true,
      title: 'Side thread',
    });
    sideId = thread.id;
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
  {:else if sideAddress}
    <Chat {ctx} address={sideAddress} {connection} sideFor={address} />
  {:else if loading}
    <p class="side-empty">Loading side thread…</p>
  {:else}
    <div class="side-start">
      <p class="side-title">Side thread</p>
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
    </div>
  {/if}
</div>

<style>
  .kk-side {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
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
  }
  .side-title {
    margin: 0;
    font-size: var(--kk-font-size-body, medium);
    font-weight: 600;
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
</style>
