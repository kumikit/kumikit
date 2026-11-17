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
  if (p.tools) lines.push('can call tools that other plugins contribute');
  if (p.limits?.memoryMb !== undefined) lines.push(`may use up to ${p.limits.memoryMb} MiB of memory per instance (default 64)`);
  if (p.limits?.callTimeoutS !== undefined) lines.push(`may compute for up to ${p.limits.callTimeoutS} s per call (default 10)`);
  if (lines.length === 0) lines.push('needs no permissions');
  return lines;
}

/** Whether a grant lets the plugin start programs outside the sandbox. */
export const startsPrograms = (p: Permissions): boolean => p.spawn.length > 0;
