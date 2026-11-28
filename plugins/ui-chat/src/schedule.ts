/**
 * Scheduling a message from the composer (06-client, Scheduled messages): quick times, the
 * cron rule each Repeat choice builds, and how a repeating entry is named. Pure, for tests.
 */
import type { QueueRepeat } from '@kumikit/protocol';

/** How often a scheduled message comes back. */
export type RepeatChoice = 'once' | 'hourly' | 'daily' | 'weekdays' | 'weekly';

export const REPEAT_CHOICES: { id: RepeatChoice; label: string }[] = [
  { id: 'once', label: 'Once' },
  { id: 'hourly', label: 'Every hour' },
  { id: 'daily', label: 'Every day' },
  { id: 'weekdays', label: 'Weekdays' },
  { id: 'weekly', label: 'Every week' },
];

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

/** A quick time next to the date field. */
export interface Preset {
  label: string;
  at: Date;
}

/** In 15 min, in 1 h, tonight at 21:00 (until then) and tomorrow at 09:00, from `now`. */
export function presets(now: Date): Preset[] {
  const out: Preset[] = [
    { label: 'In 15 min', at: new Date(now.getTime() + 15 * 60_000) },
    { label: 'In 1 h', at: new Date(now.getTime() + 60 * 60_000) },
  ];
  const tonight = new Date(now);
  tonight.setHours(21, 0, 0, 0);
  if (tonight.getTime() > now.getTime()) out.push({ label: 'Tonight 21:00', at: tonight });
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  out.push({ label: 'Tomorrow 09:00', at: tomorrow });
  return out;
}

/** A date as the value of `<input type="datetime-local">`, in local time to the minute. */
export function toLocalInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** The value of `<input type="datetime-local">` as a date, or undefined when it is not one. */
export function fromLocalInput(value: string): Date | undefined {
  const m = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/);
  if (!m) return undefined;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]));
  return Number.isNaN(d.getTime()) ? undefined : d;
}

/** The repeat rule for `choice`, from the first run's local time, in `timeZone`. */
export function repeatRule(choice: RepeatChoice, first: Date, timeZone: string | undefined): QueueRepeat | undefined {
  const m = first.getMinutes();
  const h = first.getHours();
  const cron =
    choice === 'hourly'
      ? `${m} * * * *`
      : choice === 'daily'
        ? `${m} ${h} * * *`
        : choice === 'weekdays'
          ? `${m} ${h} * * 1-5`
          : choice === 'weekly'
            ? `${m} ${h} * * ${first.getDay()}`
            : undefined;
  if (!cron) return undefined;
  return timeZone ? { cron, timeZone } : { cron };
}

/** The client's IANA time zone, if the runtime knows it. */
export function localTimeZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
  } catch {
    return undefined;
  }
}

const hhmm = (h: string, m: string) => `${h.padStart(2, '0')}:${m.padStart(2, '0')}`;

/** How a repeat rule reads: "every weekday at 09:00"; rules made elsewhere show as cron. */
export function repeatLabel(repeat: QueueRepeat): string {
  const f = repeat.cron.trim().split(/\s+/);
  if (f.length === 5 && /^\d+$/.test(f[0]!) && f[2] === '*' && f[3] === '*') {
    const [m, h, , , dow] = f as [string, string, string, string, string];
    if (h === '*' && dow === '*') return `every hour at :${m.padStart(2, '0')}`;
    if (/^\d+$/.test(h)) {
      if (dow === '*') return `every day at ${hhmm(h, m)}`;
      if (dow === '1-5') return `every weekday at ${hhmm(h, m)}`;
      if (/^[0-6]$/.test(dow)) return `every ${DAYS[Number(dow)]} at ${hhmm(h, m)}`;
    }
  }
  return `repeats \`${repeat.cron}\``;
}
