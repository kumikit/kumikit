<!-- Claude-inspired minimal conversation view: centered reading column, calm turns,
     compact tool/reasoning renderers, and a focused floating-style composer with icon
     send/cancel controls. -->
<script lang="ts">
  import { isComposing, type PluginContext, type RendererProps, type ThreadAddress, type ThreadView } from '@kumikit/ui-sdk';
  import { tick, untrack } from 'svelte';

  let { ctx, address }: { ctx: PluginContext; address: ThreadAddress } = $props();

  let view = $state.raw<ThreadView | undefined>();
  let text = $state('');
  let armedCancel = $state(false);
  let log: HTMLElement;
  let stuck = true;

  // Keep `#kk-chat-input` unique across split panes while preserving the id in single-pane mode.
  const inputId =
    typeof document !== 'undefined' && document.getElementById('kk-chat-input')
      ? `kk-chat-input-${untrack(() => address.thread).slice(0, 8)}`
      : 'kk-chat-input';

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
  const canSend = $derived(text.trim().length > 0);

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
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="log" tabindex="0" bind:this={log} onscroll={onScroll} role="log" aria-live="polite" aria-relevant="additions" aria-label="Transcript">
    <div class="column">
      {#if view && view.turns.length === 0}
        <div class="empty-state">
          <p class="empty-title">How can I help with this workspace?</p>
          <p class="empty-sub">Ask a question, request a code change, or run a tool. Press Enter to send.</p>
        </div>
      {/if}
      {#each view?.turns ?? [] as turn (turn.id)}
        <article class="turn" aria-label="Turn">
          <div class="you">
            <span class="kk-sr-only">You</span>
            <p>{turn.input}</p>
          </div>
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
  </div>

  <div class="composer-wrap">
    <form
      class="composer"
      onsubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <label class="kk-sr-only" for={inputId}>Message</label>
      <textarea
        id={inputId}
        rows="2"
        placeholder={busy ? 'Queue a follow-up message…' : 'Message the agent…'}
        bind:value={text}
        onkeydown={onKey}
      ></textarea>
      <div class="row">
        <span class="hint" role="status">
          {#if armedCancel}
            Press Escape again to cancel the turn
          {:else if view && !view.live}
            Reconnecting… messages are sent when the core is back
          {:else}
            Enter to send · Shift+Enter for new line
          {/if}
        </span>
        <div class="controls">
          {#if busy}
            <button
              type="button"
              class="action-btn stop"
              aria-label="Cancel turn"
              title="Stop running turn"
              onclick={() => run('thread.cancel')}
            >
              <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                <rect x="3.5" y="3.5" width="9" height="9" rx="1.5" />
              </svg>
            </button>
          {/if}
          <button
            type="submit"
            class="action-btn send"
            disabled={!canSend}
            aria-label="Send"
            title="Send message (Enter)"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <line x1="8" y1="12.5" x2="8" y2="3.5" />
              <polyline points="4 7.25 8 3.25 12 7.25" />
            </svg>
          </button>
        </div>
      </div>
    </form>
  </div>
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
    padding: 20px 20px 12px;
  }
  .column {
    max-width: 760px;
    margin: 0 auto;
    width: 100%;
    display: flex;
    flex-direction: column;
  }
  .empty-state {
    display: grid;
    gap: 6px;
    padding: 48px 16px;
    text-align: center;
  }
  .empty-title {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .empty-sub {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .turn {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-bottom: 26px;
  }
  .you {
    align-self: flex-end;
    max-width: 82%;
    padding: 10px 14px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .you p {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .queued {
    opacity: 0.72;
    margin-bottom: 10px;
  }
  .who {
    display: block;
    margin-bottom: 2px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .agent {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .item {
    min-width: 0;
  }
  .working,
  .usage {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .outcome {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-state-warning, CanvasText);
  }
  .outcome.failed {
    color: var(--kk-state-danger, CanvasText);
  }
  .composer-wrap {
    padding: 8px 20px 14px;
    background: var(--kk-surface-panel, Canvas);
    flex: none;
  }
  .composer {
    max-width: 760px;
    margin: 0 auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-input, Field);
    box-shadow: var(--kk-shadow-card, none);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer:focus-within {
    border-color: var(--kk-border-strong, Highlight);
  }
  textarea {
    width: 100%;
    min-height: 44px;
    max-height: 220px;
    resize: vertical;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--kk-text-primary, FieldText);
    font: inherit;
    line-height: var(--kk-line-height-base, normal);
  }
  textarea:focus {
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .hint {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }
  .action-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-medium, 0);
    cursor: pointer;
    transition:
      background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out),
      opacity var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .action-btn.send {
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
  }
  .action-btn.send:hover:not(:disabled) {
    background: var(--kk-accent-hover, Highlight);
  }
  .action-btn.send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .action-btn.stop {
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-state-danger, CanvasText);
  }
  .action-btn.stop:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  @media (prefers-reduced-motion: reduce) {
    .log {
      scroll-behavior: auto;
    }
  }
</style>
