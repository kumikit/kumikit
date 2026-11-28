<!-- Schedule a message (06-client, Scheduled messages): a popover above the composer's clock
     button. The message (the draft, or a nudge to continue), quick times, a date and time,
     and Repeat; "More options…" offers the command triggers. Sends `thread.defer`. -->
<script lang="ts">
  import type { QueueTrigger } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import { onMount, untrack } from 'svelte';
  import { fromLocalInput, localTimeZone, presets, REPEAT_CHOICES, repeatRule, toLocalInput, type RepeatChoice } from './schedule';

  let {
    draft,
    onschedule,
    onclose,
    now = () => new Date(),
  }: {
    /** The composer's draft; scheduling it clears the composer. */
    draft: string;
    onschedule: (text: string, trigger: QueueTrigger) => void;
    onclose: () => void;
    /** The clock, for tests. */
    now?: () => Date;
  } = $props();

  // The draft and quick-time presets are captured once when the popover opens.
  let message = $state(untrack(() => draft.trim() || 'Continue where you left off.'));
  const quick = untrack(() => presets(now()));
  let at = $state(toLocalInput(quick[1]!.at));
  let repeat = $state<RepeatChoice>('once');
  let more = $state(false);
  let kind = $state<'at' | 'after_command' | 'poll_command'>('at');
  let command = $state('');
  let problem = $state<string | undefined>();
  let box: HTMLTextAreaElement | undefined = $state();

  onMount(() => box?.focus());

  function trigger(): QueueTrigger | string {
    if (kind !== 'at') {
      const argv = command.trim().split(/\s+/).filter(Boolean);
      if (argv.length === 0) return 'Enter the command to wait for.';
      return kind === 'after_command' ? { kind, argv, timeoutS: 600 } : { kind, argv, everyS: 30, maxTries: 20 };
    }
    const when = fromLocalInput(at);
    if (!when) return 'Enter a date and time.';
    if (when.getTime() <= now().getTime()) return 'Choose a time in the future.';
    const rule = repeatRule(repeat, when, localTimeZone());
    return rule ? { kind: 'at', time: when.toISOString(), repeat: rule } : { kind: 'at', time: when.toISOString() };
  }

  function submit() {
    const text = message.trim();
    if (!text) {
      problem = 'Write the message to send.';
      return;
    }
    const t = trigger();
    if (typeof t === 'string') {
      problem = t;
      return;
    }
    problem = undefined;
    onschedule(text, t);
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return;
    }
    if (e.key === 'Enter') {
      const target = e.target as HTMLElement | null;
      const inInput = target?.tagName === 'INPUT';
      if (inInput || e.ctrlKey || e.metaKey) {
        e.preventDefault();
        e.stopPropagation();
        submit();
      }
    }
  }
</script>

<div class="schedule" role="dialog" aria-label="Schedule a message" tabindex="-1" onkeydown={onKey}>
  <div class="title">
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="8" cy="8" r="5.5" />
      <polyline points="8 4.8 8 8 10.4 9.4" />
    </svg>
    <span>Schedule a message</span>
  </div>

  <label class="field">
    <span class="label">Message</span>
    <textarea bind:this={box} class="kk-input" rows="3" bind:value={message} oninput={() => (problem = undefined)}></textarea>
  </label>

  {#if kind === 'at'}
    <div class="presets" role="group" aria-label="Quick times">
      {#each quick as p (p.label)}
        {@const value = toLocalInput(p.at)}
        <button
          type="button"
          class="chip"
          class:selected={at === value}
          aria-pressed={at === value}
          onclick={() => {
            at = value;
            problem = undefined;
          }}
        >
          {p.label}
        </button>
      {/each}
    </div>
    <div class="row">
      <label class="field grow">
        <span class="label">At</span>
        <input class="kk-input" type="datetime-local" bind:value={at} oninput={() => (problem = undefined)} />
      </label>
      <label class="field">
        <span class="label">Repeat</span>
        <select class="kk-input" bind:value={repeat}>
          {#each REPEAT_CHOICES as r (r.id)}<option value={r.id}>{r.label}</option>{/each}
        </select>
      </label>
    </div>
  {/if}

  {#if more}
    <label class="field">
      <span class="label">Send</span>
      <select class="kk-input" bind:value={kind} onchange={() => (problem = undefined)}>
        <option value="at">At a time</option>
        <option value="after_command">After a command exits</option>
        <option value="poll_command">When a command passes</option>
      </select>
    </label>
    {#if kind !== 'at'}
      <label class="field">
        <span class="label">Command</span>
        <input class="kk-input" type="text" placeholder="cargo test" bind:value={command} oninput={() => (problem = undefined)} />
      </label>
    {/if}
  {/if}

  {#if problem}<p class="problem" role="alert">{problem}</p>{/if}

  <div class="actions">
    {#if !more}
      <button type="button" class="link" onclick={() => (more = true)}>More options…</button>
    {/if}
    <span class="spacer"></span>
    <button type="button" class="kk-button" onclick={onclose}>Cancel</button>
    <button type="button" class="kk-button primary" onclick={submit}>Schedule</button>
  </div>
</div>

<style>
  .schedule {
    position: absolute;
    right: 0;
    bottom: calc(100% + 8px);
    z-index: 30;
    display: grid;
    gap: 10px;
    width: min(340px, 86vw);
    padding: 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-popover, none);
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    text-align: left;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .field {
    display: grid;
    gap: 4px;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .label {
    color: var(--kk-text-muted, GrayText);
  }
  textarea {
    resize: vertical;
    font: inherit;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    padding: 3px 10px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, ButtonFace);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .chip.selected {
    background: var(--kk-surface-selected, Highlight);
    border-color: var(--kk-accent, Highlight);
    color: var(--kk-text-primary, CanvasText);
  }
  .row {
    display: flex;
    align-items: end;
    gap: 8px;
  }
  .problem {
    margin: 0;
    color: var(--kk-state-danger, CanvasText);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .spacer {
    flex: 1;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--kk-text-secondary, LinkText);
    font: inherit;
    cursor: pointer;
  }
  .link:hover {
    color: var(--kk-text-primary, LinkText);
    text-decoration: underline;
  }
</style>
