/**
 * Applies the active theme's tokens as `--kk-*` custom properties (08-themes). Missing
 * tokens come from the fallback theme's variant of the same appearance. The platform
 * modes are written after the theme, so a theme cannot override them.
 */
import { TOKENS, type TokenName } from '@kumikit/ui-sdk';

export type Appearance = 'light' | 'dark';

export interface ThemeVariant {
  /** The theme plugin's id. */
  theme: string;
  /** The variant id inside the theme. */
  id: string;
  /** Display name. */
  name: string;
  appearance: Appearance;
  tokens: Partial<Record<TokenName, string>>;
}

/** What the user chose: a variant (`<theme>:<variant>`), or none to follow the
 * appearance with the fallback theme. */
export interface ThemeChoice {
  variant?: string;
  appearance: 'system' | Appearance;
  /** Forces reduced motion even if the system does not ask for it. */
  reduceMotion?: boolean;
}

export const variantKey = (v: ThemeVariant): string => `${v.theme}:${v.id}`;

/** `surface.panel` → `--kk-surface-panel`. */
export const cssVar = (name: string): string => `--kk-${name.replaceAll('.', '-')}`;

const group = (g: string) => TOKENS.filter((t) => t.group === g).map((t) => t.name);

/** Rules no theme can override (08-themes, Platform modes). */
export function platformRules(forceReducedMotion: boolean): string {
  const set = (names: readonly string[], value: string) => names.map((n) => `${cssVar(n)}:${value};`).join('');
  const motion = set(['motion.duration.short', 'motion.duration.medium'], '0ms');
  const surfaces = group('surface');
  const texts = ['text.primary', 'text.secondary', 'code.text'];
  return [
    `@media (prefers-contrast: more){:root{${cssVar('border.subtle')}:var(${cssVar('border.strong')});${set(group('elevation'), 'none')}}}`,
    `@media (prefers-reduced-transparency: reduce){:root{${cssVar('overlay.opacity')}:1;${cssVar('overlay.blur')}:0px;}}`,
    `@media (prefers-reduced-motion: reduce){:root{${motion}}}`,
    forceReducedMotion ? `:root{${motion}}` : '',
    `@media (forced-colors: active){:root{${set(surfaces, 'Canvas')}${set(['code.background'], 'Canvas')}${set(texts, 'CanvasText')}${set(['text.muted'], 'GrayText')}${set(['text.link'], 'LinkText')}${set(['accent', 'accent.hover', 'focus'], 'Highlight')}${set(['text.on-accent'], 'HighlightText')}${set(group('border'), 'CanvasText')}${set(group('elevation'), 'none')}}}`,
  ].join('\n');
}

export class ThemeEngine {
  variants = $state.raw<ThemeVariant[]>([]);
  /** Resolved tokens of what is shown now. */
  tokens = $state.raw<Partial<Record<TokenName, string>>>({});
  appearance = $state<Appearance>('light');
  /** The variant shown, if any. */
  shown = $state<string | undefined>();
  fallbackTheme: string | undefined;
  #listeners = new Set<() => void>();
  #sheet: CSSStyleSheet | undefined;

  constructor(private readonly doc?: Document) {}

  setVariants(variants: ThemeVariant[]): void {
    this.variants = variants;
  }

  onChange(fn: () => void): () => void {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  }

  /** Works out the tokens for a choice; `systemDark` is the OS appearance. */
  resolve(choice: ThemeChoice, systemDark: boolean): { tokens: Partial<Record<TokenName, string>>; appearance: Appearance; shown?: string } {
    const chosen = choice.variant ? this.variants.find((v) => variantKey(v) === choice.variant) : undefined;
    const appearance: Appearance = chosen?.appearance ?? (choice.appearance === 'system' ? (systemDark ? 'dark' : 'light') : choice.appearance);
    const fallback =
      this.variants.find((v) => v.theme === this.fallbackTheme && v.appearance === appearance) ??
      this.variants.find((v) => v.theme === this.fallbackTheme);
    const shown = chosen ?? fallback;
    const result: { tokens: Partial<Record<TokenName, string>>; appearance: Appearance; shown?: string } = {
      tokens: { ...(fallback?.tokens ?? {}), ...(chosen?.tokens ?? {}) },
      appearance,
    };
    if (shown) result.shown = variantKey(shown);
    return result;
  }

  /** The stylesheet text for resolved tokens. */
  css(tokens: Partial<Record<TokenName, string>>, appearance: Appearance, reduceMotion: boolean): string {
    const decls = Object.entries(tokens)
      .map(([n, v]) => `${cssVar(n)}:${v};`)
      .join('');
    return `:root{color-scheme:${appearance};${decls}}\n${platformRules(reduceMotion)}`;
  }

  /** Applies a choice to the document. */
  apply(choice: ThemeChoice, systemDark: boolean): void {
    const r = this.resolve(choice, systemDark);
    this.tokens = r.tokens;
    this.appearance = r.appearance;
    this.shown = r.shown;
    const text = this.css(r.tokens, r.appearance, choice.reduceMotion ?? false);
    const doc = this.doc;
    if (doc && 'adoptedStyleSheets' in doc && typeof CSSStyleSheet === 'function') {
      // A constructed sheet: the CSP allows it without inline styles.
      if (!this.#sheet) {
        this.#sheet = new CSSStyleSheet();
        doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, this.#sheet];
      }
      this.#sheet.replaceSync(text);
      doc.documentElement.dataset.appearance = r.appearance;
    }
    for (const l of [...this.#listeners]) l();
  }
}
