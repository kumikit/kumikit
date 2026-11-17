<!-- A thread as a conversation: turns, their items (drawn by whichever plugin renders
     each kind), queued messages, and the composer. -->
<script lang="ts">
  import { isComposing, type PluginContext, type RendererProps, type ThreadAddress, type ThreadView } from '@kumikit/ui-sdk';
  import { tick } from 'svelte';

  let { ctx, address }: { ctx: PluginContext; address: ThreadAddress } = $props();

  let view = $state.raw<ThreadView | undefined>();
  let text = $state('');
  let armedCancel = $state(false);
  let log: HTMLElement;
  let stuck = true;

  $effect(() => ctx.threads.watch(address, (v) => (view = v)));

  // Keep the newest output in sight while the reader is at the bottom.
  $effect(() => {
    void view;
    if (!stuck || !log) return;
    void tick().then(() => {
      if (log) log.scrollTop = log.scrollHeight;
    });
  });

  const run = (command: string, args?: unknown) => ctx.commands.run(command, args);
  const busy = $derived(view?.status === 'running' || view?.status === 'awaiting_approval');

  function item(el: HTMLElement, props: RendererProps) {
    let unmount = ctx.ui.renderItem(el, props);
    return {
      update(next: RendererProps) {
        if (next.item === props.item) return;
        props = next;
        if (typeof unmount === 'function') unmount();
        el.replaceChildren();
        unmount = ctx.ui.renderItem(el, next);
      },
      destroy() {
        if (typeof unmount === 'function') unmount();
      },
    };
  }

  function send() {
    const message = text.trim();
    if (!message) return;
    text = '';
    stuck = true;
    void run('thread.send', { address, text: message });
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    } else if (e.key === 'Escape' && busy) {
      e.preventDefault();
      if (armedCancel) {
        armedCancel = false;
        void run('thread.cancel');
      } else {
        armedCancel = true;
      }
    } else {
      armedCancel = false;
    }
  }

  function onScroll() {
    stuck = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  }

  const outcomeText: Record<string, string> = { done: '', cancelled: 'Cancelled', failed: 'Failed' };
</script>

<div class="kk-chat">
  <div class="log" bind:this={log} onscroll={onScroll} role="log" aria-live="polite" aria-relevant="additions" aria-label="Transcript">
    {#if view && view.turns.length === 0}
      <p class="empty">Send a message to start.</p>
    {/if}
    {#each view?.turns ?? [] as turn (turn.id)}
      <article class="turn" aria-label="Turn">
        <div class="you"><span class="who">You</span><p>{turn.input}</p></div>
        <div class="agent">
          {#each turn.items as it, i (i)}
            <div class="item" use:item={{ item: it, address, run }}></div>
          {/each}
          {#if !turn.ended && turn.items.length === 0}
            <p class="working" aria-label="Working">Working…</p>
          {/if}
          {#if turn.outcome && turn.outcome !== 'done'}
            <p class="outcome {turn.outcome}">{outcomeText[turn.outcome]}{turn.error ? `: ${turn.error}` : ''}</p>
          {/if}
          {#if turn.usage}
            <p class="usage">{turn.usage.inputTokens} in · {turn.usage.outputTokens} out{turn.usage.cost !== undefined ? ` · $${turn.usage.cost.toFixed(4)}` : ''}</p>
          {/if}
        </div>
      </article>
    {/each}
    {#each view?.queued ?? [] as q, i (i)}
      <div class="you queued"><span class="who">Queued</span><p>{q}</p></div>
    {/each}
  </div>
  <form class="composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <label class="kk-sr-only" for="kk-chat-input">Message</label>
    <textarea
      id="kk-chat-input"
      rows="3"
      placeholder={busy ? 'Messages sent now start when this turn ends' : 'Message the agent (Enter to send, Shift+Enter for a new line)'}
      bind:value={text}
      onkeydown={onKey}
    ></textarea>
    <div class="row">
      <span class="hint" role="status">{armedCancel ? 'Press Escape again to cancel the turn' : view && !view.live ? 'Reconnecting… messages are sent when the core is back' : ''}</span>
      <button type="submit" class="kk-button primary">Send</button>
    </div>
  </form>
</div>

<style>
  .kk-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .log {
    flex: 1;
    overflow: auto;
    padding: 16px 24px;
  }
  .empty {
    color: var(--kk-text-muted, GrayText);
  }
  .turn {
    display: grid;
    gap: 8px;
    margin-bottom: 20px;
  }
  .you {
    justify-self: end;
    max-width: 80%;
    padding: 8px 12px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-accent-tint, Canvas);
  }
  .you p {
    margin: 0;
    white-space: pre-wrap;
  }
  .queued {
    opacity: 0.7;
    margin-bottom: 8px;
  }
  .who {
    display: block;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .agent {
    display: grid;
    gap: 8px;
  }
  .working,
  .usage {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .outcome {
    margin: 0;
    color: var(--kk-state-warning, CanvasText);
  }
  .outcome.failed {
    color: var(--kk-state-danger, CanvasText);
  }
  .composer {
    display: grid;
    gap: 6px;
    padding: 10px 24px 14px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-panel, Canvas);
  }
  textarea {
    width: 100%;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font: inherit;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .hint {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  @media (prefers-reduced-motion: reduce) {
    .log {
      scroll-behavior: auto;
    }
  }
</style>
