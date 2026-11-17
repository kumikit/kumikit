import { describe, expect, it } from 'vitest';

import { contrast } from '../src/theme/contrast';
import { ThemeEngine, cssVar, platformRules, type ThemeVariant } from '../src/theme/engine.svelte';
import { checkTokens, isValid } from '../src/theme/validate';

describe('token values', () => {
  it('accept the value of each type', () => {
    expect(isValid('color', '#1a1b26')).toBe(true);
    expect(isValid('color', 'rgba(10, 20, 30, 0.5)')).toBe(true);
    expect(isValid('color', 'hsl(220 30% 20%)')).toBe(true);
    expect(isValid('length', '8px')).toBe(true);
    expect(isValid('length', '0')).toBe(true);
    expect(isValid('font-list', '"Inter", system-ui, sans-serif')).toBe(true);
    expect(isValid('duration', '120ms')).toBe(true);
    expect(isValid('easing', 'cubic-bezier(0.2, 0, 0, 1)')).toBe(true);
    expect(isValid('shadow', '0 1px 2px rgba(0,0,0,0.2), 0 4px 8px #0003')).toBe(true);
    expect(isValid('number', '0.95')).toBe(true);
  });

  it('reject anything else, including injection attempts', () => {
    for (const [type, v] of [
      ['color', 'red; background: url(https://x)'],
      ['color', 'red'],
      ['color', 'expression(alert(1))'],
      ['length', '8px} body{display:none'],
      ['length', '10vh'],
      ['font-list', 'Inter; color: red'],
      ['font-list', '"a\\"b"'],
      ['shadow', '0 0 2px url(x)'],
      ['duration', '1h'],
      ['color', '#12/**/3456'],
    ] as const) {
      expect(isValid(type, v), `${type} ${v}`).toBe(false);
    }
  });

  it('report unknown tokens and bad values but keep the good ones', () => {
    const r = checkTokens({ $schema: 'x', accent: '#7aa2f7', 'surface.panel': 'blue', nope: '#fff' });
    expect(r.tokens).toEqual({ accent: '#7aa2f7' });
    expect(r.errors).toEqual(['surface.panel: not a valid color', 'unknown token nope']);
  });
});

describe('contrast', () => {
  it('matches the WCAG ratios', () => {
    expect(contrast('#000', '#fff')).toBeCloseTo(21, 1);
    expect(contrast('#777', '#fff')).toBeCloseTo(4.48, 1);
    expect(contrast('#fff', '#fff')).toBeCloseTo(1, 5);
  });
});

describe('the engine', () => {
  const base = (appearance: 'light' | 'dark'): ThemeVariant => ({
    theme: 'base',
    id: appearance,
    name: appearance,
    appearance,
    tokens: { accent: appearance === 'dark' ? '#88f' : '#00f', 'surface.panel': appearance === 'dark' ? '#000' : '#fff' },
  });
  const small: ThemeVariant = { theme: 'small', id: 'night', name: 'Night', appearance: 'dark', tokens: { accent: '#f0f' } };

  function engine() {
    const e = new ThemeEngine();
    e.fallbackTheme = 'base';
    e.setVariants([base('light'), base('dark'), small]);
    return e;
  }

  it('follows the system appearance with the fallback theme', () => {
    const e = engine();
    expect(e.resolve({ appearance: 'system' }, true)).toMatchObject({ appearance: 'dark', shown: 'base:dark' });
    expect(e.resolve({ appearance: 'system' }, false).tokens.accent).toBe('#00f');
    expect(e.resolve({ appearance: 'dark' }, false).appearance).toBe('dark');
  });

  it('fills a small theme from the fallback of the same appearance', () => {
    const r = engine().resolve({ variant: 'small:night', appearance: 'system' }, false);
    expect(r.appearance).toBe('dark');
    expect(r.tokens).toEqual({ accent: '#f0f', 'surface.panel': '#000' });
  });

  it('writes custom properties and platform rules after them', () => {
    const e = engine();
    const css = e.css({ accent: '#00f' }, 'light', false);
    expect(css.startsWith(':root{color-scheme:light;--kk-accent:#00f;}')).toBe(true);
    expect(css.indexOf('prefers-reduced-motion')).toBeGreaterThan(css.indexOf('--kk-accent'));
    expect(cssVar('font.size.base')).toBe('--kk-font-size-base');
  });

  it('can force reduced motion', () => {
    expect(platformRules(true)).toContain(':root{--kk-motion-duration-short:0ms;');
    expect(platformRules(false)).not.toContain('\n:root{');
  });
});
