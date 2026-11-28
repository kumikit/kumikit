/**
 * Pure helpers for kumikit.ui-usage: picking the window to show, formatting numbers and
 * times, and laying out the activity heatmap. No DOM, so they are unit-tested directly.
 */
import type { DailyUsage, LimitWindow, ModelUsageRow, UsageLimits } from '@kumikit/protocol';

/** How full a window is: below 80 %, from 80 %, from 95 %. */
export type Level = 'ok' | 'warn' | 'danger';

export function level(usedPercent: number): Level {
  if (usedPercent >= 95) return 'danger';
  if (usedPercent >= 80) return 'warn';
  return 'ok';
}

/** Whether a window covers `model`: windows without a model scope cover every model;
 * a scoped window covers model ids that contain its scope (case-insensitive). */
export function appliesTo(window: LimitWindow, model: string | undefined): boolean {
  if (!window.model) return true;
  if (!model) return false;
  return model.toLowerCase().includes(window.model.toLowerCase());
}

/** The window closest to its limit among those that cover `model`; ties keep the
 * provider's order. */
export function tightest(limits: UsageLimits | undefined, model: string | undefined): LimitWindow | undefined {
  let best: LimitWindow | undefined;
  for (const w of limits?.windows ?? []) {
    if (!appliesTo(w, model)) continue;
    if (!best || w.usedPercent > best.usedPercent) best = w;
  }
  return best;
}

/** `26%`, `4.5%`, `100%`. */
export function percent(p: number): string {
  const v = Math.max(0, Math.min(100, p));
  return `${v < 10 && v % 1 !== 0 ? v.toFixed(1) : Math.round(v)}%`;
}

/** A short label for the header meter: `5h` for "5 hours", the first word otherwise. */
export function shortLabel(label: string): string {
  const hours = /^(\d+)\s*hours?$/i.exec(label.trim());
  if (hours) return `${hours[1]}h`;
  return label.trim().split(/\s+/)[0] ?? label;
}

/** `560`, `25.5K`, `31.9M`, `1.6B`. */
export function tokens(n: number): string {
  const units: [number, string][] = [
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'K'],
  ];
  for (const [size, unit] of units) {
    if (n >= size) {
      const v = n / size;
      return `${v >= 100 ? Math.round(v) : v.toFixed(1).replace(/\.0$/, '')}${unit}`;
    }
  }
  return String(Math.round(n));
}

/** `29,731,435`. */
export function exactTokens(n: number): string {
  return Math.round(n).toLocaleString('en-US');
}

/** `$626.59`, `<$0.01`, `$0.00`; a dash when unknown. */
export function cost(c: number | undefined): string {
  if (c === undefined) return '—';
  if (c > 0 && c < 0.01) return '<$0.01';
  return `$${c.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** When a window resets, relative to `now`: `2:59 PM` today, `Mon 2:59 PM` within a
 * week, `Nov 28` later. */
export function resetTime(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const time = at.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  if (sameDay(at, now)) return time;
  const days = (at.getTime() - now.getTime()) / 86_400_000;
  if (days > 0 && days < 7) return `${at.toLocaleDateString('en-US', { weekday: 'short' })} ${time}`;
  return at.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/** `12:01 PM` today, `Nov 27, 12:01 PM` otherwise. */
export function checkedTime(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const time = at.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  return sameDay(at, now) ? time : `${at.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${time}`;
}

/** The client's offset from UTC in minutes, east positive, as `usage.summary` takes it. */
export function tzOffsetMinutes(now: Date = new Date()): number {
  return -now.getTimezoneOffset();
}

/** A local date as `YYYY-MM-DD`. */
export function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** One heatmap cell. */
export interface Cell {
  date: string;
  tokens: number;
  /** 0 for no usage, 1–4 by share of the busiest day. */
  level: 0 | 1 | 2 | 3 | 4;
  /** After today: shown empty. */
  future: boolean;
}

/** A heatmap column (a week, Sunday first) and the month it starts, when it starts one. */
export interface Week {
  cells: Cell[];
  month?: string;
}

/** The weeks ending with today's, oldest first, each Sunday to Saturday. A week that
 * holds the first day of a month carries that month's name. */
export function heatmap(daily: DailyUsage[], today: Date, weeks: number): Week[] {
  const byDate = new Map(daily.map((d) => [d.date, d.tokens]));
  const max = Math.max(0, ...daily.map((d) => d.tokens));
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate() - today.getDay() - (weeks - 1) * 7);
  const todayKey = isoDate(today);
  const out: Week[] = [];
  for (let w = 0; w < weeks; w++) {
    const cells: Cell[] = [];
    let month: string | undefined;
    for (let d = 0; d < 7; d++) {
      const day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + w * 7 + d);
      const date = isoDate(day);
      const n = byDate.get(date) ?? 0;
      const share = max > 0 ? n / max : 0;
      const lvl = (n === 0 ? 0 : share > 0.75 ? 4 : share > 0.5 ? 3 : share > 0.25 ? 2 : 1) as Cell['level'];
      cells.push({ date, tokens: n, level: lvl, future: date > todayKey });
      if (day.getDate() === 1) month = day.toLocaleDateString('en-US', { month: 'short' });
    }
    out.push(month ? { cells, month } : { cells });
  }
  return out;
}

/** A date from `YYYY-MM-DD` for display: `Sat, Nov 28, 2026`. */
export function longDate(date: string): string {
  const [y, m, d] = date.split('-').map(Number);
  if (!y || !m || !d) return date;
  return new Date(y, m - 1, d).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' });
}

/** Total tokens of a model row. */
export function rowTokens(r: ModelUsageRow): number {
  return r.inputTokens + r.outputTokens;
}
