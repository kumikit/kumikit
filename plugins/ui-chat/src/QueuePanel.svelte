<!-- The thread's queue above the composer (ADR 0027): messages waiting for the running
     turn, in order. Each row can be edited, removed, sent now, and reordered by dragging
     or with Alt+Up/Down. A held queue (after Stop, a failure or a restart) says why and
     offers Resume. Every change goes to the core; the list follows the log. -->
<script lang="ts">
  import { isComposing, type QueueEntry, type QueueHold, type QueueTrigger } from '@kumikit/ui-sdk';
  import { tick } from 'svelte';
  import { repeatLabel } from './schedule';

  let {
    queue,
    hold,
    notSteered = [],
    busy,
    canSteer = false,
    disabled = false,
    run,
  }: {
    queue: QueueEntry[];
    hold?: QueueHold | undefined;
    notSteered?: string[];
    busy: boolean;
    canSteer?: boolean;
    disabled?: boolean;
    /** Runs a kernel command with the thread's address added. */
    run: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  } = $props();

  const HOLD_LABEL: Record<QueueHold, string> = {
    stopped: 'Paused after you stopped the run',
    failed: 'Paused after the run failed',
    restarted: 'Paused after the core restarted',
    limited: 'Paused after reaching a usage limit',
  };

  let editing = $state<string | undefined>();
  let draft = $state('');
  let dragged = $state<string | undefined>();
  let editor: HTMLTextAreaElement | undefined = $state();
  let list: HTMLOListElement | undefined = $state();
  let nowMs = $state(Date.now());

  // Tick countdown labels every 30s only while at least one `at` trigger is visible.
  $effect(() => {
    const hasAt = queue.some((q) => q.trigger?.kind === 'at');
    if (!hasAt) return;
    nowMs = Date.now();
    const id = setInterval(() => {
      nowMs = Date.now();
    }, 30_000);
    return () => clearInterval(id);
  });

  function triggerLabel(trigger: QueueTrigger, currentMs: number): string {
    switch (trigger.kind) {
      case 'at': {
        const target = Date.parse(trigger.time);
        let when: string;
        if (!Number.isNaN(target)) {
          const diffMin = Math.round((target - currentMs) / 60_000);
          if (diffMin <= 0) when = 'Due now';
          else if (diffMin < 60) when = `In ${diffMin}m`;
          else {
            const hours = Math.floor(diffMin / 60);
            const mins = diffMin % 60;
            when = mins > 0 ? `In ${hours}h ${mins}m` : `In ${hours}h`;
          }
        } else {
          when = `At ${trigger.time}`;
        }
        return trigger.repeat ? `${when} · ${repeatLabel(trigger.repeat)}` : when;
      }
      case 'after_turn':
        return `After thread ${trigger.thread.slice(0, 8)}${trigger.onlyOnSuccess ? ' (if ok)' : ''}`;
      case 'after_command':
        return `After \`${trigger.argv.join(' ')}\``;
      case 'poll_command':
        return `Until \`${trigger.argv.join(' ')}\` passes`;
    }
  }

  function firstLine(entry: QueueEntry): string {
    const line = entry.input.text.split('\n').find((l) => l.trim() !== '');
    return line ?? '(attachments only)';
  }

  function attachmentCount(entry: QueueEntry): number {
    return entry.input.attachments?.length ?? 0;
  }

  async function startEdit(entry: QueueEntry) {
    editing = entry.id;
    draft = entry.input.text;
    await tick();
    editor?.focus();
  }

  function cancelEdit() {
    editing = undefined;
  }

  async function saveEdit(entry: QueueEntry) {
    const text = draft.trim();
    editing = undefined;
    if (text === entry.input.text.trim()) return;
    if (!text && attachmentCount(entry) === 0) {
      await run('thread.queue.remove', { entry: entry.id });
      return;
    }
    await run('thread.queue.update', { entry: entry.id, text });
  }

  function onEditKey(e: KeyboardEvent, entry: QueueEntry) {
    if (isComposing(e)) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void saveEdit(entry);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelEdit();
    }
  }

  /** Moves `id` one step up or down; `before` is the entry it then precedes. */
  async function step(id: string, by: -1 | 1) {
    const i = queue.findIndex((q) => q.id === id);
    const target = i + by;
    if (i < 0 || target < 0 || target >= queue.length) return;
    // Moving down puts it before the entry two steps on (or at the end).
    const before = by < 0 ? queue[target]!.id : queue[target + 1]?.id;
    await run('thread.queue.move', before ? { entry: id, before } : { entry: id });
    await tick();
    list?.querySelector<HTMLElement>(`[data-entry="${CSS.escape(id)}"]`)?.focus();
  }

  function onRowKey(e: KeyboardEvent, entry: QueueEntry) {
    if (!e.altKey || disabled) return;
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      void step(entry.id, e.key === 'ArrowUp' ? -1 : 1);
    }
  }

  async function dropOn(target: QueueEntry) {
    const id = dragged;
    dragged = undefined;
    if (!id || id === target.id) return;
    const from = queue.findIndex((q) => q.id === id);
    const to = queue.findIndex((q) => q.id === target.id);
    // Dragged down it lands after the target, dragged up before it.
    const before = from < to ? queue[to + 1]?.id : target.id;
    await run('thread.queue.move', before ? { entry: id, before } : { entry: id });
  }

  const sendNowTitle = $derived(busy ? 'Stop the run and send this next' : 'Send this now');
</script>

{#if queue.length > 0}
  <section class="queue" aria-label="Queued messages">
    <header class="head">
      <strong>Queued</strong>
      <span class="count" aria-label={`${queue.length} queued`}>{queue.length}</span>
      {#if hold}
        <span class="hold" role="status">{HOLD_LABEL[hold]}</span>
        <button type="button" class="link" {disabled} onclick={() => run('thread.queue.resume')}>
          {busy ? 'Resume after this run' : 'Resume'}
        </button>
      {:else}
        <span class="next">{busy ? 'Sends when the agent finishes' : 'Sending next…'}</span>
      {/if}
    </header>
    <ol class="rows" bind:this={list}>
      {#each queue as entry, i (entry.id)}
        {@const preview = firstLine(entry)}
        {#if editing === entry.id}
          <li class="row editing">
            <label class="kk-sr-only" for={`queue-edit-${entry.id}`}>Edit queued message {i + 1}</label>
            <textarea
              id={`queue-edit-${entry.id}`}
              bind:this={editor}
              bind:value={draft}
              rows="2"
              onkeydown={(e) => onEditKey(e, entry)}
            ></textarea>
            <div class="edit-actions">
              <button type="button" class="link" onclick={cancelEdit}>Cancel</button>
              <button type="button" class="save" onclick={() => saveEdit(entry)}>Save</button>
            </div>
          </li>
        {:else}
          <!-- The row is a drag source and drop target; Alt+Up/Down is its keyboard path,
               and every action is also a button. -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
          <li
            class="row"
            class:dragging={dragged === entry.id}
            data-entry={entry.id}
            tabindex="0"
            draggable={!disabled && queue.length > 1}
            aria-label={`Queued message ${i + 1}: ${preview}`}
            aria-keyshortcuts={queue.length > 1 ? 'Alt+ArrowUp Alt+ArrowDown' : undefined}
            onkeydown={(e) => onRowKey(e, entry)}
            ondragstart={(e) => {
              dragged = entry.id;
              e.dataTransfer?.setData('text/plain', entry.id);
            }}
            ondragend={() => (dragged = undefined)}
            ondragover={(e) => {
              if (dragged) e.preventDefault();
            }}
            ondrop={(e) => {
              e.preventDefault();
              void dropOn(entry);
            }}
          >
            <span class="index" aria-hidden="true">{i + 1}</span>
            <span class="text" title={entry.input.text}>{preview}</span>
            {#if entry.trigger}
              <span class="trigger-chip" title={triggerLabel(entry.trigger, nowMs)}>
                {triggerLabel(entry.trigger, nowMs)}
              </span>
            {/if}
            {#if notSteered.includes(entry.id)}
              <span class="not-steered" title="Could not steer the running turn; queued next">Not steered</span>
            {/if}
            {#if attachmentCount(entry) > 0}
              <span class="atts" title={`${attachmentCount(entry)} attachment(s)`}>+{attachmentCount(entry)}</span>
            {/if}
            {#if entry.from?.plugin}
              <span class="from" title={`Sent by ${entry.from.plugin}`}>{entry.from.plugin}</span>
            {/if}
            <span class="actions">
              {#if busy && canSteer && !entry.trigger && attachmentCount(entry) === 0}
                <button
                  type="button"
                  class="act steer"
                  {disabled}
                  title="Send this to the running agent now"
                  aria-label={`Steer: ${preview}`}
                  onclick={() => run('thread.queue.steer', { entry: entry.id })}
                >
                  Steer
                </button>
              {/if}
              <button
                type="button"
                class="act"
                {disabled}
                title={entry.trigger ? 'Run this deferred message now' : sendNowTitle}
                aria-label={`${entry.trigger ? 'Run now' : 'Send now'}: ${preview}`}
                onclick={() => run('thread.queue.sendNow', { entry: entry.id })}
              >
                {entry.trigger ? 'Run now' : 'Send now'}
              </button>
              <button
                type="button"
                class="icon"
                {disabled}
                title="Edit message"
                aria-label={`Edit queued message ${i + 1}`}
                onclick={() => startEdit(entry)}
              >
                <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="M10.5 2.5l3 3L6 13H3v-3l7.5-7.5Z" />
                </svg>
              </button>
              <button
                type="button"
                class="icon"
                {disabled}
                title="Remove message"
                aria-label={`Remove queued message ${i + 1}`}
                onclick={() => run('thread.queue.remove', { entry: entry.id })}
              >
                <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
                  <path d="M4 4l8 8M12 4l-8 8" />
                </svg>
              </button>
            </span>
          </li>
        {/if}
      {/each}
    </ol>
  </section>
{/if}

<style>
  .queue {
    max-width: 760px;
    margin: 0 auto 8px;
    width: 100%;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    color: var(--kk-text-secondary, CanvasText);
  }
  .count {
    padding: 0 6px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-text-muted, GrayText);
  }
  .next,
  .hold {
    flex: 1;
    min-width: 0;
    color: var(--kk-text-muted, GrayText);
  }
  .hold {
    color: var(--kk-state-warning, CanvasText);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 168px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: var(--kk-radius-small, 0);
    color: var(--kk-text-primary, CanvasText);
  }
  .row:hover,
  .row:focus-visible {
    background: var(--kk-surface-card, Canvas);
  }
  .row:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: -2px;
  }
  .row.dragging {
    opacity: 0.5;
  }
  .row[draggable='true'] {
    cursor: grab;
  }
  .index {
    flex: none;
    width: 1.5em;
    text-align: right;
    color: var(--kk-text-muted, GrayText);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .atts,
  .from {
    flex: none;
    color: var(--kk-text-muted, GrayText);
  }
  .not-steered {
    flex: none;
    padding: 0 6px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-state-warning, CanvasText);
  }
  .trigger-chip {
    flex: none;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 6px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-accent, Highlight);
  }
  .act.steer {
    color: var(--kk-accent, Highlight);
  }
  .actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  button {
    font: inherit;
    color: inherit;
    background: none;
    border: none;
    cursor: pointer;
  }
  button:disabled {
    cursor: default;
    opacity: 0.5;
  }
  .act,
  .link,
  .save {
    padding: 2px 8px;
    border-radius: var(--kk-radius-small, 0);
    color: var(--kk-text-secondary, CanvasText);
  }
  .act:hover:not(:disabled),
  .link:hover:not(:disabled),
  .icon:hover:not(:disabled) {
    background: var(--kk-surface-panel, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .link {
    color: var(--kk-text-link, LinkText);
  }
  .icon {
    display: inline-flex;
    padding: 4px;
    border-radius: var(--kk-radius-small, 0);
    color: var(--kk-text-muted, GrayText);
  }
  .editing {
    flex-direction: column;
    align-items: stretch;
  }
  .editing textarea {
    font: inherit;
    font-size: var(--kk-font-size-base, medium);
    color: var(--kk-text-primary, FieldText);
    background: var(--kk-surface-card, Field);
    border: 1px solid var(--kk-border-strong, FieldText);
    border-radius: var(--kk-radius-small, 0);
    padding: 6px 8px;
    resize: vertical;
  }
  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .save {
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
  }
  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
  }
</style>
