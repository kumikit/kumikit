// @vitest-environment jsdom
import type { QueueTrigger } from '@kumikit/protocol';
import type { PluginContext, RendererProps, ThreadView } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import Chat from '../src/Chat.svelte';
import QueuePanel from '../src/QueuePanel.svelte';
import SchedulePopover from '../src/SchedulePopover.svelte';
import { fromLocalInput, presets, repeatLabel, repeatRule, toLocalInput } from '../src/schedule';

const address = { connection: 'local', workspace: 'w', thread: 't' };
const base: ThreadView = { address, status: 'idle', seq: 0, turns: [], pendingApprovals: [], queued: [], queue: [], live: true };

function fakeCtx(view: ThreadView) {
  const ran: [string, unknown][] = [];
  const ctx = {
    threads: {
      watch: (_a: unknown, fn: (v: ThreadView) => void) => {
        fn(view);
        return () => {};
      },
    },
    commands: { run: async (c: string, a?: unknown) => void ran.push([c, a]) },
    ui: {
      markdown: (el: HTMLElement, t: string) => (el.textContent = t),
      renderItem: (el: HTMLElement, p: RendererProps) => {
        el.textContent = p.item.kind === 'message' ? p.item.text : p.item.kind;
      },
    },
  } as unknown as PluginContext;
  return { ctx, ran };
}

describe('schedule helpers', () => {
  it('builds cron rules for each Repeat choice', () => {
    // 2026-11-30 09:15 local time (Monday, getDay() === 1).
    const first = new Date(2026, 10, 30, 9, 15, 0);
    expect(repeatRule('once', first, 'Europe/London')).toBeUndefined();
    expect(repeatRule('hourly', first, 'Europe/London')).toEqual({ cron: '15 * * * *', timeZone: 'Europe/London' });
    expect(repeatRule('daily', first, 'Europe/London')).toEqual({ cron: '15 9 * * *', timeZone: 'Europe/London' });
    expect(repeatRule('weekdays', first, 'Europe/London')).toEqual({ cron: '15 9 * * 1-5', timeZone: 'Europe/London' });
    expect(repeatRule('weekly', first, 'Europe/London')).toEqual({ cron: '15 9 * * 1', timeZone: 'Europe/London' });
    expect(repeatRule('daily', first, undefined)).toEqual({ cron: '15 9 * * *' });
  });

  it('names standard repeat rules and falls back to cron for custom rules', () => {
    expect(repeatLabel({ cron: '5 * * * *' })).toBe('every hour at :05');
    expect(repeatLabel({ cron: '0 9 * * *' })).toBe('every day at 09:00');
    expect(repeatLabel({ cron: '30 9 * * 1-5' })).toBe('every weekday at 09:30');
    expect(repeatLabel({ cron: '0 14 * * 5' })).toBe('every Friday at 14:00');
    expect(repeatLabel({ cron: '*/15 * * * *' })).toBe('repeats `*/15 * * * *`');
  });

  it('includes Tonight 21:00 only before 21:00 and round-trips local inputs', () => {
    const afternoon = new Date(2026, 10, 28, 14, 20, 0);
    expect(presets(afternoon).map((p) => p.label)).toEqual(['In 15 min', 'In 1 h', 'Tonight 21:00', 'Tomorrow 09:00']);

    const late = new Date(2026, 10, 28, 22, 10, 0);
    expect(presets(late).map((p) => p.label)).toEqual(['In 15 min', 'In 1 h', 'Tomorrow 09:00']);

    const formatted = toLocalInput(afternoon);
    expect(formatted).toBe('2026-11-28T14:20');
    expect(fromLocalInput(formatted)?.getTime()).toBe(afternoon.getTime());
    expect(fromLocalInput('not-a-date')).toBeUndefined();
  });
});

describe('SchedulePopover', () => {
  const fixedNow = () => new Date(2026, 10, 28, 10, 0, 0);

  it('defaults to a continuation prompt when draft is empty and emits a repeating at trigger', () => {
    const scheduled: [string, QueueTrigger][] = [];
    let closed = 0;
    const target = document.createElement('div');
    const c = mount(SchedulePopover, {
      target,
      props: {
        draft: '',
        now: fixedNow,
        onschedule: (text, trigger) => scheduled.push([text, trigger]),
        onclose: () => (closed += 1),
      },
    });
    flushSync();

    const textarea = target.querySelector<HTMLTextAreaElement>('textarea')!;
    expect(textarea.value).toBe('Continue where you left off.');

    // Click Tomorrow 09:00 preset and choose Weekdays repeat.
    const tomorrowBtn = [...target.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Tomorrow 09:00')!;
    tomorrowBtn.click();
    const repeatSel = target.querySelector<HTMLSelectElement>('select')!;
    repeatSel.value = 'weekdays';
    repeatSel.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    const submitBtn = [...target.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Schedule')!;
    submitBtn.click();
    flushSync();

    expect(closed).toBe(0);
    expect(scheduled).toHaveLength(1);
    expect(scheduled[0]![0]).toBe('Continue where you left off.');
    const trigger = scheduled[0]![1];
    expect(trigger.kind).toBe('at');
    if (trigger.kind === 'at') {
      expect(trigger.repeat?.cron).toBe('0 9 * * 1-5');
    }
    unmount(c);
  });

  it('validates past time, empty message, and command triggers under More options', () => {
    const scheduled: [string, QueueTrigger][] = [];
    const target = document.createElement('div');
    const c = mount(SchedulePopover, {
      target,
      props: {
        draft: 'check CI',
        now: fixedNow,
        onschedule: (text, trigger) => scheduled.push([text, trigger]),
        onclose: () => {},
      },
    });
    flushSync();

    const submitBtn = () => [...target.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Schedule')!;
    const dtInput = target.querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
    dtInput.value = '2026-11-28T09:00';
    dtInput.dispatchEvent(new Event('input', { bubbles: true }));
    submitBtn().click();
    flushSync();
    expect(target.querySelector('[role="alert"]')?.textContent).toBe('Choose a time in the future.');
    expect(scheduled).toHaveLength(0);

    // Reveal command triggers.
    const moreBtn = [...target.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'More options…')!;
    moreBtn.click();
    flushSync();
    const selects = target.querySelectorAll<HTMLSelectElement>('select');
    const kindSel = selects[selects.length - 1]!;
    kindSel.value = 'after_command';
    kindSel.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    submitBtn().click();
    flushSync();
    expect(target.querySelector('[role="alert"]')?.textContent).toBe('Enter the command to wait for.');

    const cmdInput = target.querySelector<HTMLInputElement>('input[type="text"]')!;
    cmdInput.value = 'cargo test --workspace';
    cmdInput.dispatchEvent(new Event('input', { bubbles: true }));
    submitBtn().click();
    flushSync();

    expect(scheduled).toEqual([
      ['check CI', { kind: 'after_command', argv: ['cargo', 'test', '--workspace'], timeoutS: 600 }],
    ]);
    unmount(c);
  });

  it('closes on Escape unless IME composition is active', () => {
    let closed = 0;
    const target = document.createElement('div');
    const c = mount(SchedulePopover, {
      target,
      props: { draft: 'hi', now: fixedNow, onschedule: () => {}, onclose: () => (closed += 1) },
    });
    flushSync();

    const dialog = target.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true, cancelable: true }));
    expect(closed).toBe(0);
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(closed).toBe(1);
    unmount(c);
  });
});

describe('Chat scheduling and QueuePanel repeat display', () => {
  it('opens the popover from the clock button, closes on outside click or Escape with focus return, and schedules the draft', () => {
    const { ctx, ran } = fakeCtx(base);
    const target = document.createElement('div');
    document.body.appendChild(target);
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    const composer = target.querySelector<HTMLTextAreaElement>('textarea')!;
    composer.value = 'run nightly checks';
    composer.dispatchEvent(new Event('input', { bubbles: true }));

    const clock = target.querySelector<HTMLButtonElement>('button[aria-label="Schedule"]')!;
    clock.click();
    flushSync();
    expect(target.querySelector('[role="dialog"][aria-label="Schedule a message"]')).toBeTruthy();

    // Escape closes and returns focus to the clock button.
    target
      .querySelector<HTMLElement>('[role="dialog"][aria-label="Schedule a message"]')!
      .dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    flushSync();
    expect(target.querySelector('[role="dialog"][aria-label="Schedule a message"]')).toBeNull();
    expect(document.activeElement).toBe(clock);

    // Reopen and close via outside pointerdown.
    clock.click();
    flushSync();
    expect(target.querySelector('[role="dialog"][aria-label="Schedule a message"]')).toBeTruthy();
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    flushSync();
    expect(target.querySelector('[role="dialog"][aria-label="Schedule a message"]')).toBeNull();

    // Reopen and schedule the draft: clears composer and sends thread.defer.
    clock.click();
    flushSync();
    const dialog = target.querySelector<HTMLElement>('[role="dialog"][aria-label="Schedule a message"]')!;
    const scheduleBtn = [...dialog.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Schedule')!;
    scheduleBtn.click();
    flushSync();

    expect(composer.value).toBe('');
    expect(ran).toHaveLength(1);
    expect(ran[0]![0]).toBe('thread.defer');
    expect((ran[0]![1] as { text: string; trigger: QueueTrigger }).text).toBe('run nightly checks');
    expect((ran[0]![1] as { text: string; trigger: QueueTrigger }).trigger.kind).toBe('at');

    unmount(c);
    target.remove();
  });

  it('shows the repeat rule in QueuePanel for a repeating deferred entry', () => {
    const target = document.createElement('div');
    const future = new Date(Date.now() + 60 * 60_000).toISOString();
    const c = mount(QueuePanel, {
      target,
      props: {
        queue: [
          {
            id: 'q1',
            turn: 'u1',
            input: { text: 'morning standup summary' },
            trigger: { kind: 'at', time: future, repeat: { cron: '0 9 * * 1-5', timeZone: 'UTC' } },
          },
        ],
        busy: false,
        run: async () => {},
      },
    });
    flushSync();

    const chip = target.querySelector('.trigger-chip');
    expect(chip?.textContent?.trim()).toContain('every weekday at 09:00');
    unmount(c);
  });
});
