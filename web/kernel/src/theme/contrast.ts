/** WCAG contrast of two colors (08-themes, Validation). */

/** The color as [r, g, b, a] in 0–255 (alpha 0–1), for #hex and rgb()/rgba(). */
export function parseColor(v: string): [number, number, number, number] | undefined {
  const s = v.trim().toLowerCase();
  if (s.startsWith('#')) {
    let h = s.slice(1);
    if (h.length <= 4) h = [...h].map((c) => c + c).join('');
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
  }
  const m = /^rgba?\(([^)]+)\)$/.exec(s);
  if (!m) return undefined;
  const parts = m[1]!.split(/[\s,/]+/).filter(Boolean).map((p) => (p.endsWith('%') ? parseFloat(p) / 100 : parseFloat(p)));
  const [r = 0, g = 0, b = 0, a = 1] = parts;
  return [r, g, b, a];
}

function luminance([r, g, b]: [number, number, number, number]): number {
  const ch = (c: number) => {
    const x = c / 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

/** The contrast ratio of `fg` on `bg` (1–21). A translucent `fg` is blended onto `bg`. */
export function contrast(fg: string, bg: string): number {
  const f = parseColor(fg);
  const b = parseColor(bg);
  if (!f || !b) return 1;
  const a = f[3];
  const blended: [number, number, number, number] = [f[0] * a + b[0] * (1 - a), f[1] * a + b[1] * (1 - a), f[2] * a + b[2] * (1 - a), 1];
  const [l1, l2] = [luminance(blended), luminance(b)];
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

/** Text tokens and the surfaces they are used on, with the WCAG AA minimum: the shared list in
 * `web/sdk/theme-rules.json`, which `kumikit theme check` reads too. */
import { CONTRAST_PAIRS } from '@kumikit/ui-sdk';

export { CONTRAST_PAIRS };

/** The pairs whose contrast is below WCAG AA in these tokens, in words; pairs with a token
 * the theme leaves to the fallback are not checked. */
export function contrastWarnings(tokens: Readonly<Partial<Record<string, string>>>): string[] {
  const out: string[] = [];
  for (const [fg, bg, min] of CONTRAST_PAIRS) {
    const f = tokens[fg];
    const b = tokens[bg];
    if (f === undefined || b === undefined) continue;
    const ratio = contrast(f, b);
    if (ratio + 1e-9 < min) out.push(`${fg} on ${bg} has contrast ${ratio.toFixed(2)}, below ${min}`);
  }
  return out;
}
