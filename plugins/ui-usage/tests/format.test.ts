import type { LimitWindow } from '@kumikit/protocol';
import { describe, expect, it } from 'vitest';

import { appliesTo, cost, heatmap, isoDate, level, percent, resetTime, shortLabel, tightest, tokens } from '../src/format';

const w = (id: string, usedPercent: number, model?: string): LimitWindow =>
  model === undefined ? { id, label: id, usedPercent } : { id, label: id, usedPercent, model };

describe('the window to show', () => {
  it('is the fullest window that covers the model', () => {
    const limits = { windows: [w('five-hour', 26), w('weekly', 19), w('weekly-opus', 60, 'opus')] };
    expect(tightest(limits, 'claude-opus-5-5')?.id).toBe('weekly-opus');
    expect(tightest(limits, 'claude-sonnet-5-5')?.id).toBe('five-hour');
    // Without a model only unscoped windows count.
    expect(tightest(limits, undefined)?.id).toBe('five-hour');
    expect(tightest(undefined, 'x')).toBeUndefined();
    expect(tightest({ windows: [] }, 'x')).toBeUndefined();
  });

  it('keeps the provider order on ties', () => {
    expect(tightest({ windows: [w('a', 10), w('b', 10)] }, 'm')?.id).toBe('a');
  });

  it('matches model scopes without case', () => {
    expect(appliesTo(w('x', 1, 'Opus'), 'claude-opus-5')).toBe(true);
    expect(appliesTo(w('x', 1, 'opus'), 'claude-sonnet')).toBe(false);
    expect(appliesTo(w('x', 1), undefined)).toBe(true);
  });
});

describe('formatting', () => {
  it('levels from 80 and 95 percent', () => {
    expect([level(79.9), level(80), level(94.9), level(95), level(100)]).toEqual(['ok', 'warn', 'warn', 'danger', 'danger']);
  });

  it('shows percentages, tokens and costs compactly', () => {
    expect([percent(26), percent(4.5), percent(120), percent(-1)]).toEqual(['26%', '4.5%', '100%', '0%']);
    expect([tokens(560), tokens(25_500), tokens(31_900_000), tokens(1_600_000_000), tokens(250_000)]).toEqual([
      '560',
      '25.5K',
      '31.9M',
      '1.6B',
      '250K',
    ]);
    expect([cost(626.59), cost(0.004), cost(0), cost(undefined), cost(1234.5)]).toEqual(['$626.59', '<$0.01', '$0.00', '—', '$1,234.50']);
  });

  it('shortens window labels for the header', () => {
    expect(shortLabel('5 hours')).toBe('5h');
    expect(shortLabel('Weekly')).toBe('Weekly');
    expect(shortLabel('Gemini weekly')).toBe('Gemini');
  });

  it('says when a window resets relative to now', () => {
    const now = new Date(2026, 10, 28, 12, 0);
    expect(resetTime(new Date(2026, 10, 28, 14, 59).toISOString(), now)).toBe('2:59 PM');
    expect(resetTime(new Date(2026, 10, 30, 14, 59).toISOString(), now)).toBe('Mon 2:59 PM');
    expect(resetTime(new Date(2026, 11, 20, 9, 0).toISOString(), now)).toBe('Dec 20');
    expect(resetTime('soon', now)).toBe('soon');
  });
});

describe('the heatmap', () => {
  it('lays out weeks ending today, Sunday first, with levels by the busiest day', () => {
    const today = new Date(2026, 10, 28); // a Saturday
    const daily = [
      { date: '2026-11-28', tokens: 100 },
      { date: '2026-11-27', tokens: 20 },
      { date: '2026-11-02', tokens: 60 },
    ];
    const weeks = heatmap(daily, today, 5);
    expect(weeks).toHaveLength(5);
    const last = weeks[4]!.cells;
    expect(last[0]!.date).toBe('2026-11-22');
    expect(last[6]).toMatchObject({ date: '2026-11-28', level: 4, future: false });
    expect(last[5]).toMatchObject({ date: '2026-11-27', level: 1 });
    const nov2 = weeks.flatMap((x) => x.cells).find((c) => c.date === '2026-11-02');
    expect(nov2?.level).toBe(3);
    expect(weeks.find((x) => x.month === 'Nov')).toBeDefined();
  });

  it('leaves the days after today empty', () => {
    const today = new Date(2026, 10, 25); // a Wednesday
    const week = heatmap([], today, 1)[0]!;
    expect(week.cells.map((c) => c.future)).toEqual([false, false, false, false, true, true, true]);
    expect(isoDate(today)).toBe('2026-11-25');
  });
});
