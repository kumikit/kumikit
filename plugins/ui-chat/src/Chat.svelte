<!-- Claude-inspired minimal conversation view: centered reading column, calm turns with
     attachment previews, compact tool/reasoning renderers, and a focused composer card
     with clipboard/file attachments plus a compact footer showing the selected model and
     permission-mode padlock popover underneath. -->
<script lang="ts">
  import type { HandoffMode, ModelInfo, ProviderInfo, QueueTrigger } from '@kumikit/protocol';
  import {
    isComposing,
    type Connection,
    type PermissionMode,
    type PluginContext,
    type RendererProps,
    type ThreadAddress,
    type ThreadSwitchView,
    type ThreadView,
  } from '@kumikit/ui-sdk';
  import { tick, untrack } from 'svelte';
  import {
    attachmentImageUrl,
    filesFromClipboard,
    formatBytes,
    fromPickedAttachments,
    readAttachments,
    toProtocolAttachments,
    type PendingAttachment,
  } from './attachments';
  import BranchPicker from './BranchPicker.svelte';
  import PermissionModePicker from './PermissionModePicker.svelte';
  import QueuePanel from './QueuePanel.svelte';

  let {
    ctx,
    address,
    connection,
    sideFor,
  }: {
    ctx: PluginContext;
    address: ThreadAddress;
    connection?: Connection;
    sideFor?: ThreadAddress;
  } = $props();

  let view = $state.raw<ThreadView | undefined>();
  const activeConn = $derived(connection ?? ctx.connection);
  const connReady = $derived(activeConn ? activeConn.ready : (view?.live ?? true));
  let text = $state('');
  let attachments = $state.raw<PendingAttachment[]>([]);
  let forkNote = $state<string | undefined>();
  let dragging = $state(false);
  let armedCancel = $state(false);
  let models = $state.raw<ModelInfo[]>([]);
  let providers = $state.raw<ProviderInfo[]>([]);
  let pendingSwitch = $state<ProviderInfo | undefined>();
  let handoffMode = $state<HandoffMode>('transcript');
  let switching = $state(false);
  let showLater = $state(false);
  let laterMode = $state<'minutes' | 'at' | 'after_command' | 'poll_command'>('minutes');
  let laterMinutes = $state(15);
  let laterTime = $state('');
  let laterCmd = $state('');
  let log: HTMLElement;
  let textarea: HTMLTextAreaElement | undefined = $state();
  let fileInput: HTMLInputElement | undefined = $state();
  let stuck = true;

  // Claim `#kk-chat-input` after DOM commit so any outgoing tab's composer has already
  // unmounted, while keeping every split pane's textarea id unique.
  const fallbackInputId = `kk-chat-input-${untrack(() => address.thread).slice(0, 8)}`;
  let inputId = $state(fallbackInputId);

  $effect(() => {
    if (!textarea || typeof document === 'undefined') return;
    const existing = document.getElementById('kk-chat-input');
    inputId = !existing || existing === textarea ? 'kk-chat-input' : fallbackInputId;
  });

  $effect(() => ctx.threads.watch(address, (v) => (view = v)));

  // Restore any preloaded edit-fork draft (text, attachments, calm notice) saved for this thread.
  $effect(() => {
    const key = `preload:${address.thread}`;
    const raw = ctx.storage?.get(key);
    if (!raw) {
      forkNote = undefined;
      return;
    }
    ctx.storage?.delete(key);
    try {
      const parsed = JSON.parse(raw) as {
        text?: string;
        attachments?: import('@kumikit/protocol').Attachment[];
        note?: string;
      };
      if (typeof parsed.text === 'string') text = parsed.text;
      if (Array.isArray(parsed.attachments) && parsed.attachments.length > 0) {
        attachments = parsed.attachments.map((att, i) => {
          const item: PendingAttachment = {
            ...att,
            id: `fork-att-${i}-${Date.now()}`,
            size: Math.floor((att.data.length * 3) / 4),
          };
          const url = attachmentImageUrl(att);
          if (url) item.previewUrl = url;
          return item;
        });
      }
      forkNote = parsed.note;
    } catch {
      // Ignore malformed preload payload.
    }
  });

  // Allow side threads or accessories to insert text into this thread's composer.
  $effect(() => {
    if (typeof window === 'undefined') return;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent<{ address?: ThreadAddress; text: string }>).detail;
      if (!detail?.text) return;
      if (
        detail.address &&
        (detail.address.connection !== address.connection ||
          detail.address.workspace !== address.workspace ||
          detail.address.thread !== address.thread)
      ) {
        return;
      }
      text = text ? `${text}\n\n${detail.text}` : detail.text;
      textarea?.focus();
    };
    window.addEventListener('kumikit:composer-insert', handler);
    return () => window.removeEventListener('kumikit:composer-insert', handler);
  });

  // Load available providers so the user can hand off to another agent when idle.
  $effect(() => {
    const conn = activeConn;
    if (!conn?.ready) return;
    let alive = true;
    conn.call('provider.list', {}).then(
      (res) => {
        if (alive && Array.isArray(res?.providers)) providers = res.providers;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  // Load the provider's model list (cached on the connection) so the footer shows the
  // human-readable model label and allows switching models when idle.
  $effect(() => {
    const provider = view?.thread?.provider;
    const conn = activeConn;
    if (!provider || !conn?.ready) return;
    let alive = true;
    conn.models(provider).then(
      (list) => {
        if (alive) models = list;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

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
  const archived = $derived(view?.status === 'archived');
  const canSend = $derived(!archived && (text.trim().length > 0 || attachments.length > 0));
  const threadProvider = $derived(view?.thread?.provider ?? '');
  const threadModel = $derived(view?.thread?.model ?? '');
  const modelInfo = $derived(models.find((m) => m.id === threadModel));
  const modelLabel = $derived(modelInfo?.label ?? (threadModel || 'Default'));
  const canSteer = $derived(modelInfo?.capabilities?.includes('steer') ?? false);
  const canSteerNow = $derived(busy && !archived && text.trim().length > 0 && attachments.length === 0);
  const permissionMode = $derived<PermissionMode>(view?.thread?.permissionMode ?? 'standard');

  function providerName(id: string): string {
    return providers.find((p) => p.id === id)?.name ?? id;
  }

  function switchesAfter(turnId?: string): ThreadSwitchView[] {
    return (view?.switches ?? []).filter((s) => s.afterTurnId === turnId);
  }

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

  async function addFiles(files: readonly File[], pasted = false) {
    if (files.length === 0) return;
    const res = await readAttachments(files, attachments, pasted);
    if (res.added.length > 0) attachments = [...attachments, ...res.added];
    if (res.error) ctx.notify(res.error, 'warning');
  }

  function removeAttachment(id: string) {
    attachments = attachments.filter((a) => a.id !== id);
  }

  function onPaste(e: ClipboardEvent) {
    const files = filesFromClipboard(e);
    if (files.length === 0) return;
    const plain = e.clipboardData?.getData('text/plain') ?? '';
    if (!plain) e.preventDefault();
    void addFiles(files, true);
  }

  async function openAttachmentPicker() {
    const picked = (await run('attachment.pick', {
      address,
      worktreePath: view?.thread?.worktreePath,
    })) as
      | { name: string; mimeType: string; data: string; size: number }[]
      | null
      | undefined;
    if (!Array.isArray(picked)) {
      fileInput?.click();
      return;
    }
    if (picked.length === 0) return;
    const res = await fromPickedAttachments(picked, attachments);
    if (res.added.length > 0) attachments = [...attachments, ...res.added];
    if (res.error) ctx.notify(res.error, 'warning');
  }

  function onFilePicked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    input.value = '';
    void addFiles(files, false);
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    const files = e.dataTransfer?.files ? Array.from(e.dataTransfer.files) : [];
    void addFiles(files, false);
  }

  function parseLaterCommand(raw: string): { text: string; trigger: QueueTrigger } | null {
    const rest = raw.slice('/later'.length).trim();
    if (!rest) return null;
    const rel = rest.match(/^(\d+)([mh])\s+([\s\S]+)$/i);
    if (rel) {
      const amount = Number(rel[1]);
      const mult = rel[2]!.toLowerCase() === 'h' ? 3_600_000 : 60_000;
      const body = rel[3]!.trim();
      if (amount > 0 && body) {
        return { text: body, trigger: { kind: 'at', time: new Date(Date.now() + amount * mult).toISOString() } };
      }
    }
    const hhmm = rest.match(/^(\d{1,2}):(\d{2})\s+([\s\S]+)$/);
    if (hhmm) {
      const h = Number(hhmm[1]);
      const m = Number(hhmm[2]);
      const body = hhmm[3]!.trim();
      if (h >= 0 && h <= 23 && m >= 0 && m <= 59 && body) {
        const target = new Date();
        target.setSeconds(0, 0);
        target.setHours(h, m);
        if (target.getTime() <= Date.now()) target.setDate(target.getDate() + 1);
        return { text: body, trigger: { kind: 'at', time: target.toISOString() } };
      }
    }
    const cmd = rest.match(/^(after|until)\s+(.+?)\s+--\s+([\s\S]+)$/i);
    if (cmd) {
      const argv = cmd[2]!.trim().split(/\s+/).filter(Boolean);
      const body = cmd[3]!.trim();
      if (argv.length > 0 && body) {
        const trigger: QueueTrigger =
          cmd[1]!.toLowerCase() === 'after'
            ? { kind: 'after_command', argv, timeoutS: 600 }
            : { kind: 'poll_command', argv, everyS: 30, maxTries: 20 };
        return { text: body, trigger };
      }
    }
    return null;
  }

  function scheduleLater() {
    const message = text.trim();
    const pending = attachments;
    if ((!message && pending.length === 0) || archived) return;
    let trigger: QueueTrigger | undefined;
    if (laterMode === 'minutes') {
      const mins = Math.max(1, Math.round(Number(laterMinutes) || 15));
      trigger = { kind: 'at', time: new Date(Date.now() + mins * 60_000).toISOString() };
    } else if (laterMode === 'at') {
      const raw = laterTime.trim();
      const hhmm = raw.match(/^(\d{1,2}):(\d{2})$/);
      if (hhmm) {
        const d = new Date();
        d.setSeconds(0, 0);
        d.setHours(Number(hhmm[1]), Number(hhmm[2]));
        if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
        trigger = { kind: 'at', time: d.toISOString() };
      } else if (raw && !Number.isNaN(Date.parse(raw))) {
        trigger = { kind: 'at', time: new Date(raw).toISOString() };
      }
    } else {
      const argv = laterCmd.trim().split(/\s+/).filter(Boolean);
      if (argv.length > 0) {
        trigger =
          laterMode === 'after_command'
            ? { kind: 'after_command', argv, timeoutS: 600 }
            : { kind: 'poll_command', argv, everyS: 30, maxTries: 20 };
      }
    }
    if (!trigger) {
      ctx.notify('Enter a valid time or command to schedule this message.', 'warning');
      return;
    }
    text = '';
    attachments = [];
    showLater = false;
    const args: {
      address: ThreadAddress;
      text: string;
      trigger: QueueTrigger;
      attachments?: ReturnType<typeof toProtocolAttachments>;
    } = { address, text: message, trigger };
    if (pending.length > 0) args.attachments = toProtocolAttachments(pending);
    void run('thread.defer', args);
  }

  function assistantText(turn: import('@kumikit/ui-sdk').TurnView): string {
    return turn.items
      .filter((it): it is Extract<typeof it, { kind: 'message' }> => it.kind === 'message')
      .map((it) => it.text)
      .join('\n\n')
      .trim();
  }

  async function forkFrom(turnId?: string) {
    await run('thread.fork', turnId ? { address, atTurn: turnId, open: true } : { address, open: true });
  }

  async function editTurn(turn: import('@kumikit/ui-sdk').TurnView) {
    const forked = (await run('thread.fork', {
      address,
      beforeTurn: turn.id,
      open: false,
    })) as import('@kumikit/protocol').Thread | undefined;
    if (!forked) return;
    ctx.storage?.set(
      `preload:${forked.id}`,
      JSON.stringify({
        text: turn.input,
        attachments: turn.attachments ?? [],
        note: 'Editing opened a fork before that prompt. Workspace files were not rewound.',
      }),
    );
    await run('thread.open', {
      address: { connection: address.connection, workspace: address.workspace, thread: forked.id },
    });
  }

  async function openBtw(question: string) {
    const conn = activeConn;
    if (!conn?.ready) return;
    let sideId: string | undefined;
    try {
      const listed = await conn.call('thread.list', { workspace: address.workspace, includeSide: true });
      sideId = (listed?.threads ?? []).find((t) => t.side && t.parent === address.thread)?.id;
    } catch {
      // Fall through to creating a fresh side thread.
    }
    if (!sideId) {
      const created = await conn.call('thread.create', {
        workspace: address.workspace,
        parent: address.thread,
        side: true,
        title: 'Side thread',
      });
      sideId = created.thread.id;
    }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('kumikit:side-thread-opened', { detail: { parent: address, side: sideId } }));
    }
    await run('inspector.open', { tab: `${ctx.id}/side` });
    if (question.trim()) {
      await run('thread.send', {
        address: { connection: address.connection, workspace: address.workspace, thread: sideId },
        text: question.trim(),
      });
    }
  }

  function copyText(content: string) {
    if (!content) return;
    if (typeof navigator !== 'undefined' && navigator.clipboard) {
      void navigator.clipboard.writeText(content).then(
        () => ctx.notify('Copied to clipboard.', 'info'),
        () => {},
      );
    }
  }

  function send() {
    const message = text.trim();
    const pending = attachments;
    if ((!message && pending.length === 0) || archived) return;
    if (message === '/fork') {
      text = '';
      void forkFrom();
      return;
    }
    if (message === '/btw' || message.startsWith('/btw ')) {
      text = '';
      void openBtw(message.slice(4).trim());
      return;
    }
    if (message.startsWith('/later')) {
      const parsed = parseLaterCommand(message);
      if (!parsed) {
        ctx.notify('Usage: /later 15m <msg>, /later 14:30 <msg>, /later after <cmd> -- <msg>, or /later until <cmd> -- <msg>', 'warning');
        return;
      }
      text = '';
      attachments = [];
      const args: {
        address: ThreadAddress;
        text: string;
        trigger: QueueTrigger;
        attachments?: ReturnType<typeof toProtocolAttachments>;
      } = { address, text: parsed.text, trigger: parsed.trigger };
      if (pending.length > 0) args.attachments = toProtocolAttachments(pending);
      void run('thread.defer', args);
      return;
    }
    text = '';
    attachments = [];
    stuck = true;
    const args: { address: ThreadAddress; text: string; attachments?: ReturnType<typeof toProtocolAttachments> } = {
      address,
      text: message,
    };
    if (pending.length > 0) args.attachments = toProtocolAttachments(pending);
    void run('thread.send', args);
  }

  function steer() {
    const message = text.trim();
    if (!message || archived || !busy) return;
    if (attachments.length > 0) {
      send();
      return;
    }
    text = '';
    stuck = true;
    void run('thread.steer', { address, text: message });
  }

  function onProviderChange(e: Event) {
    const select = e.currentTarget as HTMLSelectElement;
    const nextId = select.value;
    if (!nextId || nextId === threadProvider) return;
    if ((view?.turns.length ?? 0) === 0) {
      void run('thread.update', { address, provider: nextId });
      return;
    }
    const target = providers.find((p) => p.id === nextId);
    // Keep the select showing the current provider until the user confirms the handoff.
    select.value = threadProvider;
    if (!target) return;
    handoffMode = 'transcript';
    pendingSwitch = target;
  }

  async function confirmSwitch() {
    if (!pendingSwitch || switching) return;
    const target = pendingSwitch;
    const mode = handoffMode;
    switching = true;
    try {
      await run('thread.update', { address, provider: target.id, handoff: mode });
      pendingSwitch = undefined;
    } finally {
      switching = false;
    }
  }

  function cancelSwitch() {
    if (switching) return;
    pendingSwitch = undefined;
  }

  function onModelChange(e: Event) {
    const next = (e.currentTarget as HTMLSelectElement).value;
    if (!next || next === threadModel) return;
    void run('thread.update', { address, model: next });
  }

  function onPermissionChange(mode: PermissionMode) {
    void run('thread.update', { address, permissionMode: mode });
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && busy) {
      e.preventDefault();
      steer();
    } else if (e.key === 'Enter' && !e.shiftKey) {
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
      {#if view?.thread?.forkedFrom}
        <div class="fork-banner" role="note">
          Forked from an earlier thread{#if forkNote} · {forkNote}{/if}
          {#if forkNote}
            <button type="button" class="turn-action" onclick={() => (forkNote = undefined)}>Dismiss</button>
          {/if}
        </div>
      {:else if forkNote}
        <div class="fork-banner" role="note">
          {forkNote}
          <button type="button" class="turn-action" onclick={() => (forkNote = undefined)}>Dismiss</button>
        </div>
      {/if}
      {#if view && view.turns.length === 0}
        <div class="empty-state">
          <p class="empty-title">How can I help with this workspace?</p>
          <p class="empty-sub">Ask a question, request a code change, or paste a screenshot. Press Enter to send.</p>
        </div>
      {/if}
      {#each switchesAfter(undefined) as sw (sw.seq)}
        <div class="switch-divider" role="note">
          Switched from {providerName(sw.from.provider)} to {providerName(sw.to.provider)} ({sw.to.model}){sw.summaryFailed ? ' · summary failed, using transcript' : ''}
        </div>
      {/each}
      {#each view?.turns ?? [] as turn (turn.id)}
        {#if turn.purpose === 'handoff_summary'}
          <details class="summary-turn" aria-label="Handoff summary turn">
            <summary>
              Handoff summary{#if turn.provider} · {providerName(turn.provider)}{#if turn.model} ({turn.model}){/if}{/if}
            </summary>
            <div class="agent">
              {#each turn.items as it, i (i)}
                <div class="item" use:item={{ item: it, address, run }}></div>
              {/each}
              {#if turn.outcome && turn.outcome !== 'done'}
                <p class="outcome {turn.outcome}">{outcomeText[turn.outcome]}{turn.error ? `: ${turn.error}` : ''}</p>
              {/if}
            </div>
          </details>
        {:else}
          <article class="turn" aria-label="Turn">
            <div class="you">
              <span class="kk-sr-only">You</span>
              {#if turn.attachments && turn.attachments.length > 0}
                <div class="turn-atts">
                  {#each turn.attachments as att, idx (idx)}
                    {@const imgUrl = attachmentImageUrl(att)}
                    {#if imgUrl}
                      <figure class="turn-img-card">
                        <img src={imgUrl} alt={att.name} />
                        <figcaption>
                          <span>{att.name}</span>
                          {#if att.width && att.height}
                            <span class="dim">{att.width} × {att.height}</span>
                          {/if}
                        </figcaption>
                      </figure>
                    {:else}
                      <span class="turn-file-chip">{att.name}</span>
                    {/if}
                  {/each}
                </div>
              {/if}
              {#if turn.input}
                <p>{turn.input}</p>
              {/if}
              {#if !sideFor && !archived}
                <div class="turn-actions user-actions">
                  <button
                    type="button"
                    class="turn-action"
                    aria-label="Edit prompt in fork"
                    title="Fork before this message and edit it"
                    onclick={() => void editTurn(turn)}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    class="turn-action"
                    aria-label="Fork from turn"
                    title="Fork thread after this turn"
                    onclick={() => void forkFrom(turn.id)}
                  >
                    Fork
                  </button>
                </div>
              {/if}
            </div>
            <div class="agent">
              {#if turn.provider}
                <div class="turn-author" aria-label="Agent">
                  {turn.model ? `${providerName(turn.provider)} · ${turn.model}` : providerName(turn.provider)}
                </div>
              {/if}
              {#each turn.items as it, i (i)}
                <div class="item" use:item={{ item: it, address, run }}></div>
              {/each}
              {#if !turn.ended && turn.items.length === 0}
                <p class="working" aria-label="Working">Working…</p>
              {/if}
              {#if turn.outcome && turn.outcome !== 'done'}
                <p class="outcome {turn.outcome}">{outcomeText[turn.outcome]}{turn.error ? `: ${turn.error}` : ''}</p>
              {/if}
              {#if turn.ended}
                {@const reply = assistantText(turn)}
                <div class="turn-actions">
                  {#if reply}
                    <button type="button" class="turn-action" onclick={() => copyText(reply)}>Copy</button>
                  {/if}
                  {#if sideFor && reply}
                    <button
                      type="button"
                      class="turn-action primary"
                      onclick={() => void run('composer.insert', { address: sideFor, text: reply })}
                    >
                      Insert into main
                    </button>
                  {:else if !sideFor && !archived}
                    <button type="button" class="turn-action" onclick={() => void forkFrom(turn.id)}>Fork from here</button>
                  {/if}
                </div>
              {/if}
              {#if turn.usage}
                <p class="usage">{turn.usage.inputTokens} in · {turn.usage.outputTokens} out{turn.usage.cost !== undefined ? ` · $${turn.usage.cost.toFixed(4)}` : ''}</p>
              {/if}
            </div>
          </article>
        {/if}
        {#each switchesAfter(turn.id) as sw (sw.seq)}
          <div class="switch-divider" role="note">
            Switched from {providerName(sw.from.provider)} to {providerName(sw.to.provider)} ({sw.to.model}){sw.summaryFailed ? ' · summary failed, using transcript' : ''}
          </div>
        {/each}
      {/each}
    </div>
  </div>

  <div class="composer-wrap">
    {#if pendingSwitch}
      <div class="handoff-bar" role="region" aria-label="Confirm provider handoff">
        <span class="handoff-title">Hand off to {pendingSwitch.name}?</span>
        <div class="handoff-modes" role="radiogroup" aria-label="Handoff mode">
          <label>
            <input
              type="radio"
              name="handoff-mode"
              value="transcript"
              checked={handoffMode === 'transcript'}
              onchange={() => (handoffMode = 'transcript')}
              disabled={switching}
            />
            Transcript
          </label>
          <label>
            <input
              type="radio"
              name="handoff-mode"
              value="summary"
              checked={handoffMode === 'summary'}
              onchange={() => (handoffMode = 'summary')}
              disabled={switching}
            />
            Summary + transcript
          </label>
        </div>
        <div class="handoff-actions">
          <button type="button" class="handoff-btn primary" disabled={switching} onclick={() => void confirmSwitch()}>
            {switching ? 'Handing off…' : 'Hand off'}
          </button>
          <button type="button" class="handoff-btn" disabled={switching} onclick={cancelSwitch}>
            Cancel
          </button>
        </div>
      </div>
    {/if}
    {#if showLater}
      <div class="later-bar" role="region" aria-label="Send later">
        <select class="model-select" aria-label="Trigger type" bind:value={laterMode}>
          <option value="minutes">In minutes</option>
          <option value="at">At time (HH:MM)</option>
          <option value="after_command">After command exits</option>
          <option value="poll_command">When command passes</option>
        </select>
        {#if laterMode === 'minutes'}
          <input class="later-input short" type="number" min="1" max="10080" aria-label="Minutes from now" bind:value={laterMinutes} />
          <span class="later-unit">min</span>
        {:else if laterMode === 'at'}
          <input class="later-input" type="text" placeholder="14:30 or ISO time" aria-label="Target time" bind:value={laterTime} />
        {:else}
          <input class="later-input" type="text" placeholder="cargo test" aria-label="Command" bind:value={laterCmd} />
        {/if}
        <button type="button" class="handoff-btn primary" disabled={!canSend} onclick={scheduleLater}>
          Schedule
        </button>
        <button type="button" class="handoff-btn" onclick={() => (showLater = false)}>
          Cancel
        </button>
      </div>
    {/if}
    <QueuePanel
      queue={view?.queue ?? []}
      hold={view?.queueHold}
      notSteered={view?.notSteered ?? []}
      {busy}
      {canSteer}
      disabled={archived || !connReady}
      run={(command, args) => run(command, { address, ...args })}
    />
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="composer-shell"
      class:dragging
      onsubmit={(e) => {
        e.preventDefault();
        send();
      }}
      ondragover={(e) => {
        e.preventDefault();
        dragging = true;
      }}
      ondragleave={() => (dragging = false)}
      ondrop={onDrop}
    >
      {#if attachments.length > 0}
        <div class="att-strip" aria-label="Pending attachments">
          {#each attachments as att (att.id)}
            <div class="att-card">
              {#if att.previewUrl}
                <img class="att-thumb" src={att.previewUrl} alt={att.name} />
              {:else}
                <span class="att-file-ico" aria-hidden="true">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3">
                    <path d="M4 2h5.5L13 5.5V14H4V2Z" stroke-linejoin="round" />
                    <path d="M9.5 2v3.5H13" stroke-linejoin="round" />
                  </svg>
                </span>
              {/if}
              <div class="att-meta">
                <span class="att-name" title={att.name}>{att.name}</span>
                <span class="att-sub">
                  {#if att.width && att.height}
                    {att.width} × {att.height}
                  {:else}
                    {formatBytes(att.size)}
                  {/if}
                </span>
              </div>
              <button
                type="button"
                class="att-remove"
                aria-label={`Remove ${att.name}`}
                title="Remove attachment"
                onclick={() => removeAttachment(att.id)}
              >
                ×
              </button>
            </div>
          {/each}
        </div>
      {/if}

      <div class="composer-box">
        <label class="kk-sr-only" for={inputId}>Message</label>
        <textarea
          bind:this={textarea}
          id={inputId}
          rows="2"
          disabled={archived}
          placeholder={busy ? (canSteer ? 'Queue (Enter) or steer (Ctrl/⌘+Enter)…' : 'Queue a follow-up message…') : 'Ask for follow-up changes…'}
          bind:value={text}
          onkeydown={onKey}
          onpaste={onPaste}
        ></textarea>
        <div class="controls">
          <input
            bind:this={fileInput}
            type="file"
            multiple
            class="kk-sr-only"
            aria-label="Attach files"
            onchange={onFilePicked}
          />
          <button
            type="button"
            class="action-btn attach"
            disabled={archived}
            aria-label="Attach file"
            title="Attach file or paste image from clipboard"
            onclick={() => void openAttachmentPicker()}
          >
            <svg width="17" height="17" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12.5 7.2L7.3 12.4a3 3 0 0 1-4.2-4.2l5.9-5.9a2 2 0 1 1 2.8 2.8L6.2 10.7a1 1 0 1 1-1.4-1.4l4.9-4.9" />
            </svg>
          </button>
          {#if busy && canSteer}
            <button
              type="button"
              class="action-btn steer"
              disabled={!canSteerNow}
              aria-label="Steer"
              title="Steer running turn (Ctrl/⌘+Enter)"
              onclick={steer}
            >
              Steer
            </button>
          {/if}
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
            type="button"
            class="action-btn later"
            disabled={archived || !connReady}
            aria-label="Send later"
            aria-expanded={showLater}
            title="Send later (or use /later)"
            onclick={() => (showLater = !showLater)}
          >
            Later
          </button>
          <button
            type="submit"
            class="action-btn send"
            disabled={!canSend}
            aria-label="Send"
            title="Send message (Enter)"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.9"
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

      <div class="footer">
        <div class="footer-left">
          {#if providers.length > 1}
            <select
              class="model-select"
              aria-label="Thread provider"
              title={busy ? `Provider: ${providerName(threadProvider)} (wait for turn to finish to switch)` : `Provider: ${providerName(threadProvider)}`}
              value={threadProvider}
              disabled={busy || archived || !connReady || switching}
              onchange={onProviderChange}
            >
              {#if threadProvider && !providers.some((p) => p.id === threadProvider)}
                <option value={threadProvider}>{threadProvider}</option>
              {/if}
              {#each providers as p (p.id)}
                <option value={p.id}>{p.name}</option>
              {/each}
            </select>
          {/if}
          {#if models.length > 1}
            <select
              class="model-select"
              aria-label="Thread model"
              title={busy ? `Model: ${modelLabel} (wait for turn to finish to change)` : `Model: ${modelLabel}`}
              value={threadModel}
              disabled={busy || archived || !connReady}
              onchange={onModelChange}
            >
              {#if threadModel && !models.some((m) => m.id === threadModel)}
                <option value={threadModel}>{modelLabel}</option>
              {/if}
              {#each models as m (m.id)}
                <option value={m.id}>{m.label}</option>
              {/each}
            </select>
          {:else if threadModel}
            <span class="model-pill" title={`Model: ${modelLabel}`}>{modelLabel}</span>
          {/if}
          <PermissionModePicker
            value={permissionMode}
            disabled={busy || archived || !connReady}
            onChange={onPermissionChange}
          />
        </div>
        <div class="footer-right">
          {#if armedCancel}
            <span class="hint" role="status">Press Escape again to cancel the turn</span>
          {:else if view && !view.live}
            <span class="hint" role="status">Reconnecting…</span>
          {/if}
          <BranchPicker
            connection={activeConn}
            workspaceId={address.workspace}
            worktreePath={view?.thread?.worktreePath}
            disabled={busy}
            onNotify={(m, t) => ctx.notify(m, t)}
            onSelectWorktree={(path) => {
              const args: { connection: string; workspace: string; worktreePath?: string } = {
                connection: address.connection,
                workspace: address.workspace,
              };
              if (path) args.worktreePath = path;
              void run('thread.new', args);
            }}
          />
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .you p {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .turn-atts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .turn-img-card {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 240px;
  }
  .turn-img-card img {
    max-width: 240px;
    max-height: 160px;
    object-fit: cover;
    border-radius: var(--kk-radius-medium, 0);
    border: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .turn-img-card figcaption {
    display: flex;
    gap: 6px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .turn-file-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .agent {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .turn-author {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-muted, GrayText);
  }
  .switch-divider {
    margin: 6px 0 18px;
    padding: 4px 10px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    text-align: center;
  }
  .summary-turn {
    margin-bottom: 18px;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .summary-turn summary {
    cursor: pointer;
    color: var(--kk-text-muted, GrayText);
  }
  .handoff-bar {
    max-width: 760px;
    margin: 0 auto 8px;
    width: 100%;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-primary, CanvasText);
  }
  .handoff-title {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .handoff-modes {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--kk-text-secondary, CanvasText);
  }
  .handoff-modes label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }
  .handoff-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .handoff-btn {
    padding: 4px 10px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-card, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .handoff-btn.primary {
    border-color: transparent;
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
  }
  .handoff-btn:disabled {
    opacity: 0.5;
    cursor: default;
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
  .composer-shell {
    max-width: 760px;
    margin: 0 auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    box-shadow: var(--kk-shadow-card, none);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer-shell.dragging {
    border-color: var(--kk-accent, Highlight);
  }
  .att-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 10px 2px;
  }
  .att-card {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 220px;
    padding: 4px 8px 4px 5px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  .att-thumb {
    width: 32px;
    height: 32px;
    object-fit: cover;
    border-radius: var(--kk-radius-small, 0);
    flex: none;
  }
  .att-file-ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-secondary, CanvasText);
    flex: none;
  }
  .att-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .att-name {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .att-sub {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .att-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
    flex: none;
  }
  .att-remove:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .composer-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 6px 0;
    padding: 10px 12px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-input, Field);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer-box:focus-within {
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
  .controls {
    display: flex;
    align-items: center;
    justify-content: flex-end;
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
  .action-btn.attach {
    width: 32px;
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
  }
  .action-btn.attach:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .action-btn.attach:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .action-btn.send {
    width: 32px;
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
  .action-btn.steer {
    width: auto;
    padding: 0 10px;
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-accent, Highlight);
    font-size: var(--kk-font-size-small, small);
  }
  .action-btn.steer:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .action-btn.steer:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .action-btn.later {
    width: auto;
    padding: 0 8px;
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .action-btn.later:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .action-btn.later:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .later-bar {
    max-width: 760px;
    margin: 0 auto 8px;
    width: 100%;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-primary, CanvasText);
  }
  .later-input {
    flex: 1;
    min-width: 120px;
    padding: 4px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, FieldText);
    font: inherit;
  }
  .later-input.short {
    flex: 0 0 72px;
    min-width: 60px;
  }
  .later-unit {
    color: var(--kk-text-muted, GrayText);
  }
  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
  }
  .footer-left {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .footer-right {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .model-select {
    height: 24px;
    max-width: 260px;
    padding: 0 6px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .model-select:hover:not(:disabled),
  .model-select:focus-visible {
    background: var(--kk-surface-hover, ButtonFace);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .model-select:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .model-pill {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    padding: 0 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 240px;
  }
  .hint {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fork-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px;
    margin-bottom: 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .turn-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .user-actions {
    justify-content: flex-end;
    margin-top: 4px;
  }
  .turn-action {
    padding: 2px 8px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .turn-action:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .turn-action.primary {
    border-color: var(--kk-accent, Highlight);
    color: var(--kk-accent, Highlight);
  }
  @media (prefers-reduced-motion: reduce) {
    .log {
      scroll-behavior: auto;
    }
  }
</style>
