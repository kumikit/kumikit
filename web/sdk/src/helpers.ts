import type { TokenName } from './generated';

/** `var(--kk-…)` for a token: `surface.panel` → `var(--kk-surface-panel)`. */
export function tokenVar(name: TokenName): string {
  return `var(--kk-${name.replaceAll('.', '-')})`;
}

/** Whether a key event belongs to an IME composition; Enter and Escape handlers in text
 * inputs must ignore those (AGENTS.md). */
export function isComposing(event: KeyboardEvent): boolean {
  return event.isComposing || event.keyCode === 229;
}
