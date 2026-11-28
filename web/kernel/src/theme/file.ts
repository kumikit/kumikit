/**
 * Parsing and validation of single-file `.json` theme resources (08-themes, ADR 0037).
 */
import type { TokenName } from '@kumikit/ui-sdk';

import { contrastWarnings } from './contrast';
import type { Appearance } from './engine.svelte';
import { checkTokens } from './validate';

export interface ThemeResource {
  id: string;
  name: string;
  appearance: Appearance;
  tokens: Partial<Record<TokenName, string>>;
}

export interface ParsedTheme {
  theme: ThemeResource;
  /** Ignored token errors (unknown names or invalid CSS values). */
  errors: string[];
  /** WCAG AA contrast warnings. */
  warnings: string[];
}

/** Converts a theme name or file stem into a lowercase `[a-z0-9-]` slug. */
export function slugifyThemeId(raw: string): string {
  const slug = raw
    .replace(/\.json$/i, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'custom-theme';
}

const VALID_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

/**
 * Parses and validates a JSON object representing a theme file. Throws a plain-word
 * Error if the top-level structure (`name`, `appearance`, `tokens`) is invalid.
 */
export function parseThemeObject(raw: unknown, fallbackFileName?: string): ParsedTheme {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    throw new Error('The theme file must be a JSON object.');
  }
  const obj = raw as Record<string, unknown>;
  const name = typeof obj.name === 'string' ? obj.name.trim() : '';
  if (!name) {
    throw new Error('The theme file must include a non-empty "name".');
  }
  if (obj.appearance !== 'light' && obj.appearance !== 'dark') {
    throw new Error('The theme file must set "appearance" to "light" or "dark".');
  }
  const appearance: Appearance = obj.appearance;
  if (typeof obj.tokens !== 'object' || obj.tokens === null || Array.isArray(obj.tokens)) {
    throw new Error('The theme file must include a "tokens" object.');
  }
  let id: string;
  if (obj.id !== undefined) {
    if (typeof obj.id !== 'string' || !VALID_ID.test(obj.id)) {
      throw new Error('The theme "id" must use lowercase letters, digits and dashes.');
    }
    id = obj.id;
  } else {
    id = slugifyThemeId(fallbackFileName || name);
  }
  const checked = checkTokens(obj.tokens);
  const warnings = contrastWarnings(checked.tokens);
  return {
    theme: { id, name, appearance, tokens: checked.tokens },
    errors: checked.errors,
    warnings,
  };
}

/** Parses and validates raw JSON text from a theme file. */
export function parseThemeFile(text: string, fallbackFileName?: string): ParsedTheme {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    throw new Error(`Not valid JSON: ${e instanceof Error ? e.message : String(e)}`);
  }
  return parseThemeObject(raw, fallbackFileName);
}
