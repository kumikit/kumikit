/**
 * Commands from the UI kernel, UI plugins and (later) core plugins, in one palette
 * (06-client). Every kernel action is a command, so everything is reachable from the
 * keyboard.
 */
import type { CommandSpec } from '@kumikit/ui-sdk';

export interface Command extends CommandSpec {
  /** `kernel` or the contributing plugin's id. */
  source: string;
  run(args?: unknown): unknown;
}

/** Context keys a `when` clause can test. */
export type WhenContext = Record<string, boolean>;

/** `a && !b`: every key must be true (or false with `!`). An empty clause is true. */
export function whenMatches(when: string | undefined, ctx: WhenContext): boolean {
  if (!when || when.trim() === '') return true;
  return when.split('&&').every((part) => {
    const p = part.trim();
    return p.startsWith('!') ? !ctx[p.slice(1).trim()] : Boolean(ctx[p]);
  });
}

/** A fuzzy score of `query` in `text` (higher is better), or -1 if the letters do not
 * appear in order. Consecutive letters and word starts score more. */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase().replace(/\s+/g, '');
  const t = text.toLowerCase();
  if (q === '') return 0;
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found < 0) return -1;
    run = found === ti ? run + 1 : 1;
    const wordStart = found === 0 || /[\s.:/-]/.test(t[found - 1]!);
    score += run * 2 + (wordStart ? 3 : 0) - Math.min(found - ti, 3);
    ti = found + 1;
  }
  return score;
}

/** Whether a key event matches a binding such as `Mod+Shift+P`. `Mod` is Cmd on macOS
 * and Ctrl elsewhere. */
export function keyMatches(binding: string, e: KeyboardEvent, mac: boolean): boolean {
  const parts = binding.split('+').map((p) => p.trim().toLowerCase());
  const key = parts.pop() ?? '';
  const want = { ctrl: false, meta: false, shift: false, alt: false };
  for (const m of parts) {
    if (m === 'mod') want[mac ? 'meta' : 'ctrl'] = true;
    else if (m === 'ctrl' || m === 'meta' || m === 'shift' || m === 'alt') want[m] = true;
  }
  return (
    e.key.toLowerCase() === key &&
    e.ctrlKey === want.ctrl &&
    e.metaKey === want.meta &&
    e.shiftKey === want.shift &&
    e.altKey === want.alt
  );
}

export class CommandRegistry {
  commands = $state.raw<Command[]>([]);

  register(spec: CommandSpec, run: (args?: unknown) => unknown, source = 'kernel'): () => void {
    const cmd: Command = { ...spec, source, run };
    this.commands = [...this.commands.filter((c) => c.id !== spec.id), cmd];
    return () => {
      this.commands = this.commands.filter((c) => c !== cmd);
    };
  }

  /** Removes every command of a source (a plugin being disabled). */
  removeSource(source: string): void {
    this.commands = this.commands.filter((c) => c.source !== source);
  }

  async run(id: string, args?: unknown): Promise<unknown> {
    const cmd = this.commands.find((c) => c.id === id);
    if (!cmd) throw new Error(`no command ${id}`);
    return cmd.run(args);
  }

  /** Commands available in `ctx`, best match for `query` first. */
  search(query: string, ctx: WhenContext): Command[] {
    return this.commands
      .filter((c) => whenMatches(c.when, ctx))
      .map((c) => ({ c, s: Math.max(fuzzyScore(query, c.title), fuzzyScore(query, c.id) - 2) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s || a.c.title.localeCompare(b.c.title))
      .map((x) => x.c);
  }

  /** The command bound to a key event in `ctx`, if any. */
  forKey(e: KeyboardEvent, ctx: WhenContext, mac: boolean): Command | undefined {
    return this.commands.find((c) => c.keybinding && whenMatches(c.when, ctx) && keyMatches(c.keybinding, e, mac));
  }
}
