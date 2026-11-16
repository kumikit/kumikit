/** Focus handling for dialogs: focus moves in, Tab stays in, and focus returns to where
 * it was when the dialog closes (06-client, keyboard first). */
const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function trapFocus(node: HTMLElement): { destroy(): void } {
  const before = document.activeElement as HTMLElement | null;
  const first = node.querySelector<HTMLElement>('[data-autofocus]') ?? node.querySelector<HTMLElement>(FOCUSABLE);
  first?.focus();
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Tab') return;
    const items = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) return;
    const [head, tail] = [items[0]!, items.at(-1)!];
    if (e.shiftKey && document.activeElement === head) {
      tail.focus();
      e.preventDefault();
    } else if (!e.shiftKey && document.activeElement === tail) {
      head.focus();
      e.preventDefault();
    }
  };
  node.addEventListener('keydown', onKey);
  return {
    destroy() {
      node.removeEventListener('keydown', onKey);
      before?.focus?.();
    },
  };
}
