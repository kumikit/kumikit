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
  import {
    collectTurnFiles,
    formatElapsed,
    formatMentionPath,
    groupTurnItems,
    matchSlashCommands,
    parseMentionAt,
    type MentionMatch,
    type SlashMenuEntry,
  } from './transcript';

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
  let nowMs = $state(Date.now());
  let slashIndex = $state(0);
  let mention = $state<MentionMatch | undefined>();
  let mentionFiles = $state.raw<string[]>([]);
  let mentionIndex = $state(0);
  let historyIndex = $state(-1);
  let findOpen = $state(false);
  let findQuery = $state('');
  let findIndex = $state(0);
  let findInput: HTMLInputElement | undefined = $state();
  let showTurnCheck = $state(false);
  let turnCheckCmd = $state('');
  let turnCheckEnabled = $state(false);
  let lastCheckedTurnId = '';
  let stuck = $state(true);
  let log: HTMLElement;
  let textarea: HTMLTextAreaElement | undefined = $state();
  let fileInput: HTMLInputElement | undefined = $state();

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

  // Restore any preloaded edit-fork draft (text, attachments, calm notice) or saved draft.
  $effect(() => {
    const preloadKey = `preload:${address.thread}`;
    const rawPreload = ctx.storage?.get(preloadKey);
    if (rawPreload) {
      ctx.storage?.delete(preloadKey);
      try {
        const parsed = JSON.parse(rawPreload) as {
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
        return;
      } catch {
        // Ignore malformed preload payload.
      }
    }
    forkNote = undefined;
    const savedDraft = ctx.storage?.get(`draft:${address.thread}`);
    text = savedDraft ?? '';
    historyIndex = -1;
  });

  // Persist per-thread composer draft in client storage.
  $effect(() => {
    const key = `draft:${address.thread}`;
    const current = text;
    if (current.length > 0) ctx.storage?.set(key, current);
    else ctx.storage?.delete(key);
  });

  // Load opt-in per-workspace post-turn check setting (off by default).
  $effect(() => {
    const raw = ctx.storage?.get(`turncheck:${address.workspace}`);
    if (!raw) {
      turnCheckEnabled = false;
      turnCheckCmd = '';
      return;
    }
    try {
      const parsed = JSON.parse(raw) as { enabled?: boolean; cmd?: string };
      turnCheckEnabled = Boolean(parsed.enabled);
      turnCheckCmd = typeof parsed.cmd === 'string' ? parsed.cmd : '';
    } catch {
      turnCheckEnabled = false;
      turnCheckCmd = '';
    }
  });

  function saveTurnCheck(enabled: boolean, cmd: string) {
    turnCheckEnabled = enabled;
    turnCheckCmd = cmd;
    if (!enabled && !cmd.trim()) {
      ctx.storage?.delete(`turncheck:${address.workspace}`);
    } else {
      ctx.storage?.set(`turncheck:${address.workspace}`, JSON.stringify({ enabled, cmd }));
    }
  }

  // Opt-in post-turn check: after a turn ends `done` with file changes, schedule a deferred
  // `after_command` (`onlyOnFailure: true`) with an editable fix prompt.
  $effect(() => {
    if (!turnCheckEnabled || !turnCheckCmd.trim() || sideFor || archived) return;
    const turns = view?.turns ?? [];
    const last = turns[turns.length - 1];
    if (!last || last.outcome !== 'done' || !last.ended || last.id === lastCheckedTurnId) return;
    if (collectTurnFiles(last.items).length === 0) return;
    lastCheckedTurnId = last.id;
    const argv = turnCheckCmd.trim().split(/\s+/);
    if (argv.length === 0) return;
    void run('thread.defer', {
      address,
      text: 'The post-turn check failed. Fix the reported issue.',
      trigger: { kind: 'after_command', argv, timeoutS: 300, onlyOnFailure: true },
    });
  });

  // Tick elapsed timer once per second only while a turn is running.
  $effect(() => {
    const hasOpen = (view?.turns ?? []).some((t) => !t.ended);
    if (!hasOpen) return;
    nowMs = Date.now();
    const id = setInterval(() => {
      nowMs = Date.now();
    }, 1000);
    return () => clearInterval(id);
  });

  const slashEntries = $derived<SlashMenuEntry[]>(
    view?.status === 'archived' ? [] : matchSlashCommands(text, view?.commands),
  );
  $effect(() => {
    void slashEntries.length;
    if (slashIndex >= slashEntries.length) slashIndex = 0;
  });

  // Fetch matching workspace files whenever an `@query` mention is active.
  $effect(() => {
    const m = mention;
    const conn = activeConn;
    if (!m || !conn?.ready || archived) {
      mentionFiles = [];
      return;
    }
    let alive = true;
    const params: { workspace: string; path?: string; query?: string; limit: number } = {
      workspace: address.workspace,
      limit: 20,
    };
    if (view?.thread?.worktreePath) params.path = view.thread.worktreePath;
    if (m.query) params.query = m.query;
    conn.call('workspace.files.list', params).then(
      (res) => {
        if (!alive) return;
        mentionFiles = Array.isArray(res?.files) ? res.files : [];
        mentionIndex = 0;
      },
      () => {
        if (alive) mentionFiles = [];
      },
    );
    return () => {
      alive = false;
    };
  });

  const findMatches = $derived.by(() => {
    const q = findQuery.trim().toLowerCase();
    if (!findOpen || !q) return [] as string[];
    const ids: string[] = [];
    for (const t of view?.turns ?? []) {
      const hay = `${t.input}\n${assistantText(t)}`.toLowerCase();
      if (hay.includes(q)) ids.push(t.id);
    }
    return ids;
  });

  $effect(() => {
    if (findIndex >= findMatches.length) findIndex = 0;
  });

  function scrollToFindMatch(idx: number) {
    const id = findMatches[idx];
    if (!id || !log) return;
    const el = log.querySelector<HTMLElement>(`[data-turn-id="${id}"]`);
    el?.scrollIntoView({ block: 'center' });
  }

  function insertAtCursor(snippet: string) {
    if (!textarea) {
      text = text ? `${text}${snippet}` : snippet;
      return;
    }
    const start = textarea.selectionStart ?? text.length;
    const end = textarea.selectionEnd ?? text.length;
    text = `${text.slice(0, start)}${snippet}${text.slice(end)}`;
    void tick().then(() => {
      if (!textarea) return;
      const pos = start + snippet.length;
      textarea.focus();
      textarea.setSelectionRange(pos, pos);
    });
  }

  function mountAccessories(el: HTMLElement) {
    const conn = activeConn;
    if (!conn || !ctx.ui.renderAccessories) return {};
    const dispose = ctx.ui.renderAccessories(el, {
      address,
      connection: conn,
      insert: insertAtCursor,
    });
    return {
      destroy() {
        if (typeof dispose === 'function') dispose();
      },
    };
  }

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
    const trimmed = question.trim();
    if (!trimmed) {
      await run('inspector.open', { tab: `${ctx.id}/side` });
      return;
    }
    const title = trimmed.length > 48 ? `${trimmed.slice(0, 45)}…` : trimmed;
    const created = await conn.call('thread.create', {
      workspace: address.workspace,
      parent: address.thread,
      side: true,
      title,
    });
    const sideId = created.thread.id;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('kumikit:side-thread-opened', {
          detail: { parent: address, side: sideId, thread: created.thread },
        }),
      );
    }
    await run('inspector.open', { tab: `${ctx.id}/side` });
    await run('thread.send', {
      address: { connection: address.connection, workspace: address.workspace, thread: sideId },
      text: trimmed,
    });
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
    if (message.startsWith('/rename ')) {
      const nextTitle = message.slice(8).trim();
      if (nextTitle) {
        text = '';
        void run('thread.update', { address, title: nextTitle });
      }
      return;
    }
    if (message.startsWith('/model ')) {
      const nextModel = message.slice(7).trim();
      if (nextModel) {
        text = '';
        void run('thread.update', { address, model: nextModel });
      }
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

  function pickSlash(entry: SlashMenuEntry) {
    text = `/${entry.name} `;
    slashIndex = 0;
    textarea?.focus();
  }

  function pickMention(path: string) {
    if (!mention) return;
    const formatted = `${formatMentionPath(path)} `;
    const before = text.slice(0, mention.start);
    const after = text.slice(mention.end);
    text = `${before}${formatted}${after}`;
    const pos = before.length + formatted.length;
    mention = undefined;
    mentionFiles = [];
    void tick().then(() => {
      if (!textarea) return;
      textarea.focus();
      textarea.setSelectionRange(pos, pos);
    });
  }

  function updateMentionFromCursor() {
    if (!textarea) {
      mention = undefined;
      return;
    }
    mention = parseMentionAt(text, textarea.selectionStart ?? text.length);
  }

  async function approveAll() {
    const list = view?.pendingApprovals ?? [];
    for (const ap of list) {
      await run('approval.respond', { address, approval: ap.id, decision: 'allow' });
    }
  }

  function onPaneKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      findOpen = true;
      void tick().then(() => findInput?.focus());
    } else if (e.key === 'Escape' && findOpen) {
      e.preventDefault();
      findOpen = false;
      findQuery = '';
    }
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (slashEntries.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        slashIndex = (slashIndex + 1) % slashEntries.length;
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        slashIndex = (slashIndex - 1 + slashEntries.length) % slashEntries.length;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        const picked = slashEntries[slashIndex];
        if (picked) pickSlash(picked);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        text = '';
        return;
      }
    }
    if (mention && mentionFiles.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        mentionIndex = (mentionIndex + 1) % mentionFiles.length;
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        mentionIndex = (mentionIndex - 1 + mentionFiles.length) % mentionFiles.length;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        const picked = mentionFiles[mentionIndex];
        if (picked) pickMention(picked);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        mention = undefined;
        mentionFiles = [];
        return;
      }
    }
    // Recall previous prompts on Up/Down when composer is empty or browsing history.
    const prompts = (view?.turns ?? []).map((t) => t.input).filter((s) => s.length > 0);
    if (e.key === 'ArrowUp' && prompts.length > 0 && (text === '' || historyIndex >= 0)) {
      e.preventDefault();
      const next = historyIndex < 0 ? prompts.length - 1 : Math.max(0, historyIndex - 1);
      historyIndex = next;
      text = prompts[next] ?? '';
      return;
    }
    if (e.key === 'ArrowDown' && historyIndex >= 0) {
      e.preventDefault();
      if (historyIndex + 1 < prompts.length) {
        historyIndex += 1;
        text = prompts[historyIndex] ?? '';
      } else {
        historyIndex = -1;
        text = '';
      }
      return;
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && busy) {
      e.preventDefault();
      steer();
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      historyIndex = -1;
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
      historyIndex = -1;
    }
  }

  function onScroll() {
    stuck = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  }

  const outcomeText: Record<string, string> = { done: '', cancelled: 'Cancelled', failed: 'Failed' };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="kk-chat" onkeydown={onPaneKey}>
  {#if findOpen}
    <div class="find-bar" role="search" aria-label="Find in thread">
      <input
        bind:this={findInput}
        class="later-input"
        type="search"
        placeholder="Find in thread…"
        aria-label="Find in thread"
        bind:value={findQuery}
        onkeydown={(e) => {
          if (isComposing(e)) return;
          if (e.key === 'Enter' && findMatches.length > 0) {
            e.preventDefault();
            findIndex = (findIndex + (e.shiftKey ? findMatches.length - 1 : 1)) % findMatches.length;
            scrollToFindMatch(findIndex);
          }
        }}
      />
      <span class="later-unit" role="status">
        {findMatches.length > 0 ? `${findIndex + 1} / ${findMatches.length}` : findQuery.trim() ? '0 matches' : ''}
      </span>
      <button
        type="button"
        class="turn-action"
        disabled={findMatches.length === 0}
        onclick={() => {
          findIndex = (findIndex + findMatches.length - 1) % findMatches.length;
          scrollToFindMatch(findIndex);
        }}
      >
        Prev
      </button>
      <button
        type="button"
        class="turn-action"
        disabled={findMatches.length === 0}
        onclick={() => {
          findIndex = (findIndex + 1) % findMatches.length;
          scrollToFindMatch(findIndex);
        }}
      >
        Next
      </button>
      <button
        type="button"
        class="turn-action"
        aria-label="Close find"
        onclick={() => {
          findOpen = false;
          findQuery = '';
        }}
      >
        ×
      </button>
    </div>
  {/if}
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
          {@const elapsed = formatElapsed(turn.started, turn.ended, nowMs)}
          {@const displayEntries = groupTurnItems(turn.items)}
          {@const changedFiles = collectTurnFiles(turn.items)}
          {@const isMatch = findMatches.includes(turn.id)}
          <article class="turn" class:find-hit={isMatch} data-turn-id={turn.id} aria-label="Turn">
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
                    class="turn-action icon-action"
                    aria-label="Edit prompt in fork"
                    title="Fork before this message and edit it"
                    onclick={() => void editTurn(turn)}
                  >
                    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M11.4 2.6a1.4 1.4 0 0 1 2 2L5.5 12.5l-3 .8.8-3 8.1-7.7Z" />
                    </svg>
                  </button>
                  <button
                    type="button"
                    class="turn-action icon-action"
                    aria-label="Fork from turn"
                    title="Fork thread after this turn"
                    onclick={() => void forkFrom(turn.id)}
                  >
                    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <circle cx="5" cy="3.5" r="1.5" />
                      <circle cx="11" cy="3.5" r="1.5" />
                      <circle cx="5" cy="12.5" r="1.5" />
                      <path d="M5 5v6M11 5v1a2 2 0 0 1-2 2H5" />
                    </svg>
                  </button>
                </div>
              {/if}
            </div>
            <div class="agent">
              {#if turn.provider || elapsed}
                <div class="turn-author" aria-label="Agent">
                  {#if turn.provider}
                    {turn.model ? `${providerName(turn.provider)} · ${turn.model}` : providerName(turn.provider)}
                  {/if}
                  {#if elapsed}
                    <span class="turn-elapsed">{turn.provider ? ` · ${elapsed}` : elapsed}</span>
                  {/if}
                </div>
              {/if}
              {#each displayEntries as entry (entry.kind === 'item' ? `i-${entry.index}` : `g-${entry.startIndex}`)}
                {#if entry.kind === 'tool-group'}
                  <details class="tool-group">
                    <summary>
                      {`${entry.items.length} tool calls${entry.failed > 0 ? ` · ${entry.failed} failed` : ''}${entry.pending > 0 ? ` · ${entry.pending} running` : ''}`}
                    </summary>
                    <div class="tool-group-body">
                      {#each entry.items as toolIt, idx (idx)}
                        <div class="item" use:item={{ item: toolIt, address, run }}></div>
                      {/each}
                    </div>
                  </details>
                {:else}
                  <div class="item" use:item={{ item: entry.item, address, run }}></div>
                {/if}
              {/each}
              {#if !turn.ended && turn.items.length === 0}
                <p class="working" aria-label="Working">Working…</p>
              {/if}
              {#if turn.outcome && turn.outcome !== 'done'}
                <p class="outcome {turn.outcome}">{outcomeText[turn.outcome]}{turn.error ? `: ${turn.error}` : ''}</p>
              {/if}
              {#if changedFiles.length > 0}
                <details class="changes-footer" aria-label="Changed files">
                  <summary>
                    {changedFiles.length} {changedFiles.length === 1 ? 'file' : 'files'} changed
                  </summary>
                  <ul class="changes-list">
                    {#each changedFiles as f (f.path)}
                      <li>
                        <details>
                          <summary>
                            <span class="change-path">{f.path}</span>
                            {#if f.added > 0 || f.removed > 0}
                              <span class="change-stat">+{f.added} -{f.removed}</span>
                            {/if}
                          </summary>
                          {#if f.diff}
                            <pre class="change-diff">{f.diff}</pre>
                          {/if}
                        </details>
                      </li>
                    {/each}
                  </ul>
                </details>
              {/if}
              {#if turn.ended}
                {@const reply = assistantText(turn)}
                <div class="turn-actions">
                  {#if reply}
                    <button
                      type="button"
                      class="turn-action icon-action"
                      aria-label="Copy reply"
                      title="Copy reply"
                      onclick={() => copyText(reply)}
                    >
                      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <rect x="5.5" y="5.5" width="8" height="8" rx="1.2" />
                        <path d="M10.5 5.5V3.7A1.2 1.2 0 0 0 9.3 2.5H3.7A1.2 1.2 0 0 0 2.5 3.7v5.6a1.2 1.2 0 0 0 1.2 1.2h1.8" />
                      </svg>
                    </button>
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
                    <button
                      type="button"
                      class="turn-action icon-action"
                      aria-label="Fork from here"
                      title="Fork thread after this turn"
                      onclick={() => void forkFrom(turn.id)}
                    >
                      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <circle cx="5" cy="3.5" r="1.5" />
                        <circle cx="11" cy="3.5" r="1.5" />
                        <circle cx="5" cy="12.5" r="1.5" />
                        <path d="M5 5v6M11 5v1a2 2 0 0 1-2 2H5" />
                      </svg>
                    </button>
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
    {#if !stuck && (view?.turns.length ?? 0) > 0}
      <div class="jump-wrap">
        <button
          type="button"
          class="turn-action"
          onclick={() => {
            stuck = true;
            if (log) log.scrollTop = log.scrollHeight;
          }}
        >
          Jump to bottom
        </button>
      </div>
    {/if}
    {#if (view?.pendingApprovals.length ?? 0) > 0}
      <div class="approval-bar" role="group" aria-label="Pending approvals">
        <div class="approval-bar-head">
          <span>
            {view!.pendingApprovals.length} pending {view!.pendingApprovals.length === 1 ? 'approval' : 'approvals'}
          </span>
          {#if view!.pendingApprovals.length > 1}
            <button type="button" class="handoff-btn primary" onclick={() => void approveAll()}>
              Approve all
            </button>
          {/if}
        </div>
        {#each view!.pendingApprovals as ap (ap.id)}
          <div class="approval-row">
            <span class="approval-summary">{ap.summary}</span>
            <div class="approval-row-actions">
              <button
                type="button"
                class="turn-action primary"
                onclick={() => void run('approval.respond', { address, approval: ap.id, decision: 'allow' })}
              >
                Allow
              </button>
              <button
                type="button"
                class="turn-action"
                onclick={() => void run('approval.respond', { address, approval: ap.id, decision: 'deny' })}
              >
                Deny
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {#if slashEntries.length > 0}
      <div class="picker-menu" role="listbox" aria-label="Slash commands">
        {#each slashEntries as entry, idx (entry.name)}
          <button
            type="button"
            role="option"
            aria-selected={idx === slashIndex}
            class="picker-item"
            class:active={idx === slashIndex}
            onclick={() => pickSlash(entry)}
          >
            <span class="picker-name">/{entry.name}{#if entry.hint} <span class="picker-hint">{entry.hint}</span>{/if}</span>
            {#if entry.description}
              <span class="picker-desc">{entry.description}</span>
            {/if}
          </button>
        {/each}
      </div>
    {:else if mention && mentionFiles.length > 0}
      <div class="picker-menu" role="listbox" aria-label="Mention file">
        {#each mentionFiles as file, idx (file)}
          <button
            type="button"
            role="option"
            aria-selected={idx === mentionIndex}
            class="picker-item"
            class:active={idx === mentionIndex}
            onclick={() => pickMention(file)}
          >
            <span class="picker-name">{file}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if view?.status === 'limited'}
      <div class="limit-bar" role="status" aria-label="Usage limit reached">
        <span class="limit-text">
          Usage limit reached{#if view.limit?.resetAt} · resumes at {view.limit.resetAt.slice(11, 16)} UTC{/if}{#if view.limit?.message} — {view.limit.message}{/if}
        </span>
        <button
          type="button"
          class="handoff-btn primary"
          onclick={() => void run('thread.queue.resume', { address })}
        >
          Resume now
        </button>
      </div>
    {/if}
    {#if showTurnCheck}
      <div class="later-bar" role="region" aria-label="Post-turn check">
        <label class="check-toggle">
          <input
            type="checkbox"
            checked={turnCheckEnabled}
            onchange={(e) => saveTurnCheck((e.currentTarget as HTMLInputElement).checked, turnCheckCmd)}
          />
          Run check after edits
        </label>
        <input
          class="later-input"
          type="text"
          placeholder="cargo test"
          aria-label="Turn check command"
          value={turnCheckCmd}
          oninput={(e) => saveTurnCheck(turnCheckEnabled, (e.currentTarget as HTMLInputElement).value)}
        />
        <button type="button" class="handoff-btn" onclick={() => (showTurnCheck = false)}>Done</button>
      </div>
    {/if}
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
          oninput={updateMentionFromCursor}
          onclick={updateMentionFromCursor}
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
          {#if !sideFor && !archived && (view?.turns.length ?? 0) > 0}
            <button
              type="button"
              class="action-btn later"
              disabled={!connReady}
              aria-label="Fork thread"
              title="Fork this thread"
              onclick={() => void forkFrom()}
            >
              <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="5" cy="3.5" r="1.5" />
                <circle cx="11" cy="3.5" r="1.5" />
                <circle cx="5" cy="12.5" r="1.5" />
                <path d="M5 5v6M11 5v1a2 2 0 0 1-2 2H5" />
              </svg>
            </button>
          {/if}
          <button
            type="button"
            class="action-btn later"
            class:active={showLater}
            disabled={archived || !connReady}
            aria-label="Send later"
            aria-expanded={showLater}
            title="Send later (or use /later)"
            onclick={() => (showLater = !showLater)}
          >
            <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <circle cx="8" cy="8" r="5.5" />
              <polyline points="8 4.8 8 8 10.4 9.4" />
            </svg>
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
          {#if !sideFor}
            <button
              type="button"
              class="turn-action icon-action"
              class:primary={turnCheckEnabled && turnCheckCmd.trim().length > 0}
              aria-label="Turn check settings"
              aria-expanded={showTurnCheck}
              title={turnCheckEnabled && turnCheckCmd.trim() ? `Post-turn check: ${turnCheckCmd}` : 'Configure post-turn check'}
              onclick={() => (showTurnCheck = !showTurnCheck)}
            >
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="8" cy="8" r="5.75" />
                <polyline points="5.5 8.2 7.2 9.9 10.7 6.3" />
              </svg>
            </button>
          {/if}
          <span class="accessories" use:mountAccessories></span>
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
    width: 30px;
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
  }
  .action-btn.later:hover:not(:disabled),
  .action-btn.later.active {
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
  .turn-action.icon-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border-color: transparent;
  }
  .turn-action.icon-action:hover {
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .turn-action.primary {
    border-color: var(--kk-accent, Highlight);
    color: var(--kk-accent, Highlight);
  }
  .turn-elapsed {
    color: var(--kk-text-muted, GrayText);
  }
  .turn.find-hit {
    outline: 1px solid var(--kk-accent, Highlight);
    border-radius: var(--kk-radius-medium, 0);
  }
  .find-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-raised, Canvas);
  }
  .jump-wrap {
    max-width: 760px;
    margin: 0 auto 6px;
    width: 100%;
    display: flex;
    justify-content: center;
  }
  .tool-group {
    margin: 6px 0;
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .tool-group > summary {
    cursor: pointer;
    color: var(--kk-text-secondary, CanvasText);
  }
  .tool-group-body {
    margin-top: 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .changes-footer {
    margin-top: 8px;
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .changes-footer > summary {
    cursor: pointer;
    color: var(--kk-text-secondary, CanvasText);
  }
  .changes-list {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .change-path {
    font-family: var(--kk-font-mono, monospace);
    color: var(--kk-text-primary, CanvasText);
  }
  .change-stat {
    margin-left: 8px;
    font-family: var(--kk-font-mono, monospace);
    color: var(--kk-text-muted, GrayText);
  }
  .change-diff {
    margin: 4px 0 0;
    padding: 6px 8px;
    overflow: auto;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
    color: var(--kk-code-text, CanvasText);
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
  }
  .approval-bar {
    max-width: 760px;
    margin: 0 auto 8px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .approval-bar-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    color: var(--kk-text-primary, CanvasText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .approval-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .approval-summary {
    color: var(--kk-text-secondary, CanvasText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .approval-row-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .picker-menu {
    max-width: 760px;
    margin: 0 auto 6px;
    width: 100%;
    max-height: 200px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  .picker-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 10px;
    border: 0;
    background: transparent;
    color: var(--kk-text-primary, CanvasText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
  }
  .picker-item:hover,
  .picker-item.active {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .picker-name {
    font-family: var(--kk-font-mono, monospace);
    white-space: nowrap;
  }
  .picker-hint {
    color: var(--kk-text-muted, GrayText);
  }
  .picker-desc {
    color: var(--kk-text-secondary, CanvasText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .accessories {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .limit-bar {
    max-width: 760px;
    margin: 0 auto 8px;
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--kk-state-warning, Mark);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .limit-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .check-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .log {
      scroll-behavior: auto;
    }
  }
</style>
