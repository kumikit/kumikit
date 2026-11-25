/**
 * Folding thread events into the view plugins render (`ThreadView` in the UI SDK).
 * Pure: a batch of events gives a new view; the old one is not changed, so Svelte and
 * plugins can compare by reference. The status rules match the kernel's
 * (`crates/kumikit-kernel/src/fold.rs`); a shared fixture checks both.
 */
import type { Approval, EventNotification, SlashCommand, ThreadStatus } from '@kumikit/protocol';
import type { ThreadAddress, ThreadSwitchView, ThreadView, TranscriptItem, TurnView } from '@kumikit/ui-sdk';

/** A view with nothing folded in yet. */
export function emptyView(address: ThreadAddress): ThreadView {
  return {
    address,
    status: 'idle',
    seq: 0,
    turns: [],
    pendingApprovals: [],
    queued: [],
    queue: [],
    live: false,
  };
}

/** The status after an event, as the kernel derives it. `archived` never comes from
 * the log. */
export function nextStatus(
  current: ThreadStatus,
  event: EventNotification,
  pendingAfter: number,
): ThreadStatus {
  switch (event.kind) {
    case 'turn.started':
      return 'running';
    case 'approval.requested':
      return 'awaiting_approval';
    case 'approval.resolved':
      return pendingAfter === 0 ? 'running' : 'awaiting_approval';
    case 'limit.reached':
      return 'limited';
    case 'queue.resumed':
      return current === 'limited' ? 'idle' : current;
    case 'turn.ended':
      if (current === 'limited') return 'limited';
      return event.data.outcome === 'failed' ? 'failed' : 'idle';
    default:
      return current;
  }
}

/** Applies events in order. Events at or below `view.seq` are skipped, so a replay that
 * overlaps what was folded cannot duplicate anything. */
export function foldEvents(view: ThreadView, events: readonly EventNotification[]): ThreadView {
  const fresh = events.filter((e) => e.seq > view.seq && e.topic.startsWith('thread:'));
  if (fresh.length === 0) return view;
  const turns = view.turns.slice();
  const copied = new Set<number>();
  /** The turn at `index`, copied once per batch so the old view stays intact. */
  const own = (index: number): TurnView => {
    const turn = turns[index]!;
    if (copied.has(index)) return turn;
    const copy = { ...turn, items: turn.items.slice() };
    turns[index] = copy;
    copied.add(index);
    return copy;
  };
  const turnIndex = (id: string): number => {
    for (let i = turns.length - 1; i >= 0; i--) if (turns[i]!.id === id) return i;
    return -1;
  };
  const withTurn = (id: string, f: (t: TurnView) => void): void => {
    const i = turnIndex(id);
    if (i >= 0) f(own(i));
  };
  /** Replaces item `i` of a turn with a changed copy. */
  const update = <K extends TranscriptItem['kind']>(
    t: TurnView,
    i: number,
    change: (item: Extract<TranscriptItem, { kind: K }>) => TranscriptItem,
  ): void => {
    t.items[i] = change(t.items[i] as Extract<TranscriptItem, { kind: K }>);
  };

  let pending: Approval[] = view.pendingApprovals;
  let queue = view.queue;
  let queueHold = view.queueHold;
  let notSteered = view.notSteered;
  let switches: ThreadSwitchView[] | undefined = view.switches;
  let commands: SlashCommand[] | undefined = view.commands;
  let limit = view.limit;
  let status = view.status;
  let seq = view.seq;

  const dropNotSteered = (id: string) => {
    if (notSteered && notSteered.includes(id)) {
      const next = notSteered.filter((x) => x !== id);
      notSteered = next.length > 0 ? next : undefined;
    }
  };

  for (const e of fresh) {
    seq = e.seq;
    switch (e.kind) {
      case 'turn.started': {
        limit = undefined;
        const turn: TurnView = { id: e.data.turn, input: e.data.input.text, started: e.time, items: [] };
        if (e.data.input.attachments && e.data.input.attachments.length > 0) {
          turn.attachments = e.data.input.attachments;
        }
        if (e.data.provider !== undefined) turn.provider = e.data.provider;
        if (e.data.model !== undefined) turn.model = e.data.model;
        if (e.data.purpose !== undefined) turn.purpose = e.data.purpose;
        turns.push(turn);
        const entry = e.data.entry;
        if (entry !== undefined) {
          queue = queue.filter((q) => q.id !== entry);
          dropNotSteered(entry);
        }
        break;
      }
      case 'input.steered':
        withTurn(e.data.turn, (t) => t.items.push({ kind: 'steer', text: e.data.input.text }));
        break;
      case 'thread.switched': {
        const afterTurnId = turns.length > 0 ? turns[turns.length - 1]!.id : undefined;
        const entry: ThreadSwitchView = {
          seq: e.seq,
          from: e.data.from,
          to: e.data.to,
          handoff: e.data.handoff,
        };
        if (afterTurnId !== undefined) entry.afterTurnId = afterTurnId;
        if (e.data.summaryFailed) entry.summaryFailed = true;
        switches = switches ? [...switches, entry] : [entry];
        break;
      }
      case 'handoff.delivered':
        withTurn(e.data.turn, (t) => {
          t.items.push({
            kind: 'handoff',
            from: e.data.from,
            to: e.data.to,
            messages: e.data.messages,
            truncated: Boolean(e.data.truncated),
            rendered: e.data.rendered,
          });
        });
        break;
      case 'commands.available':
        commands = e.data.commands;
        break;
      case 'limit.reached': {
        const nextLimit: { resetAt?: string; message: string } = { message: e.data.message };
        if (e.data.resetAt !== undefined) nextLimit.resetAt = e.data.resetAt;
        limit = nextLimit;
        break;
      }
      case 'queue.added':
        queue = e.data.front ? [e.data.entry, ...queue] : [...queue, e.data.entry];
        if (e.data.reason === 'not_steered') {
          notSteered = notSteered ? [...notSteered.filter((x) => x !== e.data.entry.id), e.data.entry.id] : [e.data.entry.id];
        }
        break;
      case 'queue.updated': {
        const changed = e.data.entry;
        queue = queue.map((q) => (q.id === changed.id ? changed : q));
        break;
      }
      case 'queue.removed': {
        const id = e.data.entry;
        queue = queue.filter((q) => q.id !== id);
        dropNotSteered(id);
        break;
      }
      case 'queue.moved': {
        const byId = new Map(queue.map((q) => [q.id, q]));
        const moved = e.data.order.flatMap((id) => byId.get(id) ?? []);
        const listed = new Set(e.data.order);
        queue = [...moved, ...queue.filter((q) => !listed.has(q.id))];
        break;
      }
      case 'queue.held':
        queueHold = e.data.reason;
        break;
      case 'queue.resumed':
        queueHold = undefined;
        limit = undefined;
        break;
      case 'text.delta':
        withTurn(e.data.turn, (t) => {
          const i = t.items.length - 1;
          const last = t.items[i];
          if (last?.kind === 'message' && !last.done) {
            update<'message'>(t, i, (m) => ({ ...m, text: m.text + e.data.text }));
          } else {
            t.items.push({ kind: 'message', text: e.data.text, done: false });
          }
        });
        break;
      case 'text.done':
        withTurn(e.data.turn, (t) => {
          const i = t.items.length - 1;
          const last = t.items[i];
          if (last?.kind === 'message' && !last.done) {
            t.items[i] = { kind: 'message', text: e.data.text, done: true };
          } else {
            t.items.push({ kind: 'message', text: e.data.text, done: true });
          }
        });
        break;
      case 'reasoning.delta':
        withTurn(e.data.turn, (t) => {
          const i = t.items.length - 1;
          if (t.items[i]?.kind === 'reasoning') {
            update<'reasoning'>(t, i, (r) => ({ ...r, text: r.text + e.data.text }));
          } else {
            t.items.push({ kind: 'reasoning', text: e.data.text });
          }
        });
        break;
      case 'tool.started':
        withTurn(e.data.turn, (t) => {
          t.items.push({
            kind: 'tool',
            call: e.data.call,
            name: e.data.name,
            input: e.data.input,
            output: '',
          });
        });
        break;
      case 'tool.output':
        withTurn(e.data.turn, (t) => {
          const i = t.items.findLastIndex((x) => x.kind === 'tool' && x.call === e.data.call);
          if (i >= 0) update<'tool'>(t, i, (x) => ({ ...x, output: x.output + e.data.chunk }));
        });
        break;
      case 'tool.finished':
        withTurn(e.data.turn, (t) => {
          const i = t.items.findLastIndex((x) => x.kind === 'tool' && x.call === e.data.call);
          if (i >= 0) {
            update<'tool'>(t, i, (x) => {
              const done: TranscriptItem = { ...x, ok: e.data.ok };
              if (e.data.result !== undefined) done.result = e.data.result;
              return done;
            });
          }
        });
        break;
      case 'file.changed':
        withTurn(e.data.turn, (t) => {
          const item: TranscriptItem = { kind: 'file.changed', path: e.data.path };
          if (e.data.diff !== undefined) item.diff = e.data.diff;
          t.items.push(item);
        });
        break;
      case 'approval.requested': {
        const approval = e.data.approval;
        withTurn(approval.turn, (t) => t.items.push({ kind: 'approval', approval }));
        pending = [...pending, approval];
        break;
      }
      case 'approval.resolved': {
        const id = e.data.approval;
        pending = pending.filter((a) => a.id !== id);
        for (let ti = turns.length - 1; ti >= 0; ti--) {
          const i = turns[ti]!.items.findIndex((x) => x.kind === 'approval' && x.approval.id === id);
          if (i >= 0) {
            update<'approval'>(own(ti), i, (x) => ({ ...x, resolution: e.data.decision }));
            break;
          }
        }
        break;
      }
      case 'usage':
        withTurn(e.data.turn, (t) => {
          const usage: NonNullable<TurnView['usage']> = {
            inputTokens: e.data.inputTokens,
            outputTokens: e.data.outputTokens,
          };
          if (e.data.cost !== undefined) usage.cost = e.data.cost;
          t.usage = usage;
        });
        break;
      case 'turn.ended':
        withTurn(e.data.turn, (t) => {
          t.ended = e.time;
          t.outcome = e.data.outcome;
          if (e.data.error !== undefined) t.error = e.data.error;
        });
        break;
      case 'plugin.event': {
        // Plugin events name no turn; they belong to the latest one.
        if (turns.length > 0) {
          own(turns.length - 1).items.push({
            kind: 'plugin.event',
            plugin: e.data.plugin,
            event: e.data.kind,
            data: e.data.data,
          });
        }
        break;
      }
      default:
        // Kinds this kernel does not know are skipped (02-protocol).
        break;
    }
    status = nextStatus(status, e, pending.length);
  }
  const next: ThreadView = { ...view, turns, pendingApprovals: pending, status, seq };
  if (queue !== view.queue) {
    next.queue = queue;
    next.queued = queue.map((q) => q.input.text);
  }
  if (queueHold === undefined) delete next.queueHold;
  else next.queueHold = queueHold;
  if (notSteered === undefined) delete next.notSteered;
  else next.notSteered = notSteered;
  if (switches !== undefined) next.switches = switches;
  if (commands !== undefined) next.commands = commands;
  if (limit === undefined) delete next.limit;
  else next.limit = limit;
  return next;
}
