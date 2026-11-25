/** Permissions in plain words (04-plugin-host, Permissions), the same lines
 * `kumikit plugin add` prints; a shared fixture keeps the two in step. */
import type { Permissions } from '@kumikit/protocol';

export function describePermissions(p: Permissions): string[] {
  const lines: string[] = [];
  for (const prog of p.spawn) {
    lines.push(`can start the \`${prog}\` program, which runs with your full user access, outside the sandbox`);
  }
  for (const host of p.network) lines.push(`can connect to ${host}`);
  if (p.files === 'read_workspace') lines.push('can read files in your workspaces');
  if (p.files === 'workspace') lines.push('can read and change files in your workspaces');
  for (const s of p.secrets) lines.push(`can read the secret \`${s}\``);
  if (p.tools) lines.push('can let its agent use tools that other plugins contribute');
  const ops: Record<string, string> = {
    'workspaces.list': 'can list your workspaces',
    'threads.read': 'can read threads',
    'threads.create': 'can start threads',
    'threads.send': 'can send messages to threads',
    'threads.steer': 'can steer running turns',
    'threads.defer': 'can schedule deferred messages on threads',
    'threads.queue': 'can inspect and edit thread message queues',
    'threads.title.suggest': 'can suggest titles for threads',
    'threads.cancel': 'can cancel running turns',
    'approvals.respond': 'can answer approvals (allow or deny)',
  };
  for (const op of p.client ?? []) {
    const line = ops[op];
    if (line === undefined) return [`asks for an unknown client operation \`${op}\``];
    lines.push(line);
  }
  if (p.workspaces === 'linked') lines.push('only in the workspaces you link to it');
  if (p.workspaces === 'all') lines.push('in every workspace');
  if (p.schedules) lines.push('can run on a schedule, even when you are not using Kumikit');
  if (p.limits?.memoryMb !== undefined) lines.push(`may use up to ${p.limits.memoryMb} MiB of memory per instance (default 64)`);
  if (p.limits?.callTimeoutS !== undefined) lines.push(`may compute for up to ${p.limits.callTimeoutS} s per call (default 10)`);
  if (lines.length === 0) lines.push('needs no permissions');
  return lines;
}

/** Whether a grant lets the plugin start programs outside the sandbox. */
export const startsPrograms = (p: Permissions): boolean => p.spawn.length > 0;

/** What a UI plugin or theme needs, in plain words (07-ui-plugins). */
export function describeUiNeeds(kind: string[], ui: { isolation?: string; network?: string[]; requiresCore?: string[] }): string[] {
  if (kind.includes('theme')) return ['colors, fonts and sizes only; runs no code'];
  const lines: string[] = [];
  if (ui.isolation === 'iframe') lines.push('runs in a sandboxed panel: it sees only its own tab’s core, read-only');
  else lines.push('runs its code inside the app, with full access to the app and every connected core (asks for trust)');
  for (const h of ui.network ?? []) lines.push(`can connect to ${h}`);
  for (const r of ui.requiresCore ?? []) lines.push(`shows only on cores with ${r}`);
  return lines;
}

/** A key id shortened for a list; the full id goes in a title. */
export const shortKey = (key: string): string => (key.length > 24 ? `${key.slice(0, 20)}…` : key);
