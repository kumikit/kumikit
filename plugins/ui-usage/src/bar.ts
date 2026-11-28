/**
 * Sets an element's width from a percentage through the CSSOM. The CSP allows no inline
 * style attributes, so bars are sized from script.
 */
export function fill(node: HTMLElement, pct: number) {
  const set = (p: number) => node.style.setProperty('width', `${Math.max(0, Math.min(100, p))}%`);
  set(pct);
  return { update: set };
}
