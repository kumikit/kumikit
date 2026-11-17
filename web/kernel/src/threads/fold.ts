/**
 * Folding thread events into the view plugins render (`ThreadView` in the UI SDK).
 * Pure: a batch of events gives a new view; the old one is not changed, so Svelte and
 * plugins can compare by reference. The status rules match the kernel's
 * (`crates/kumikit-kernel/src/fold.rs`); a shared fixture checks both.
 */
import type { Approval, EventNotification, ThreadStatus } from '@kumikit/protocol';
import type { ThreadAddress, ThreadView, TranscriptItem, TurnView } from '@kumikit/ui-sdk';

/** A view with nothing folded in yet. */
export function emptyView(address: ThreadAddress): ThreadView {
  return {
    address,
    status: 'idle',
    seq: 0,
    turns: [],
    pendingApprovals: [],
    queued: [],
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
    case 'turn.ended':
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
  let queued = view.queued;
  let status = view.status;
  let seq = view.seq;

  for (const e of fresh) {
    seq = e.seq;
    switch (e.kind) {
      case 'turn.started': {
        turns.push({ id: e.data.turn, input: e.data.input.text, started: e.time, items: [] });
        if (queued[0] === e.data.input.text) queued = queued.slice(1);
        break;
      }
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
  return { ...view, turns, pendingApprovals: pending, queued, status, seq };
}
